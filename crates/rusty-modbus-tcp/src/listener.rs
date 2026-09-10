//! TCP server listener — accepts incoming Modbus/TCP connections.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::StreamExt;
use rusty_modbus_frame::mbap::MbapCodec;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::Framed;
use tracing::{debug, trace};

use crate::config::TcpServerConfig;
use crate::connect::{TcpRecvStream, TcpSink};
use crate::error::TransportError;

/// Point-in-time listener counters.
///
/// Snapshots are immutable but not transactional: counters may advance while a
/// snapshot is being collected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TcpServerMetricsSnapshot {
    /// Connections currently holding an admission reservation.
    pub active_connections: usize,
    /// TCP sockets returned by [`TcpServerListener::accept_socket`] or
    /// [`TcpServerListener::accept`], not authenticated TLS sessions.
    pub accepted_connections: usize,
    /// Connections rejected by IP access control.
    pub access_denied_connections: usize,
    /// Connections rejected because all admission slots were occupied.
    pub connection_limit_rejections: usize,
}

#[derive(Debug)]
struct TcpServerMetricsInner {
    active_connections: Arc<AtomicUsize>,
    accepted_connections: AtomicUsize,
    access_denied_connections: AtomicUsize,
    connection_limit_rejections: AtomicUsize,
}

/// Cloneable access to listener counters, including after the listener is dropped.
#[derive(Debug, Clone)]
pub struct TcpServerMetrics {
    inner: Arc<TcpServerMetricsInner>,
}

impl TcpServerMetrics {
    fn new() -> Self {
        Self {
            inner: Arc::new(TcpServerMetricsInner {
                active_connections: Arc::new(AtomicUsize::new(0)),
                accepted_connections: AtomicUsize::new(0),
                access_denied_connections: AtomicUsize::new(0),
                connection_limit_rejections: AtomicUsize::new(0),
            }),
        }
    }

    fn reserve_connection(&self, maximum: usize) -> Option<ConnectionGuard> {
        let active = &self.inner.active_connections;
        let mut current = active.load(Ordering::Relaxed);
        loop {
            if current >= maximum {
                increment_saturating(&self.inner.connection_limit_rejections);
                return None;
            }
            match active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(ConnectionGuard {
                        counter: Arc::clone(active),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    fn record_accepted(&self) {
        increment_saturating(&self.inner.accepted_connections);
    }

    fn record_access_denied(&self) {
        increment_saturating(&self.inner.access_denied_connections);
    }

    /// Collect the current counter values.
    #[must_use]
    pub fn snapshot(&self) -> TcpServerMetricsSnapshot {
        TcpServerMetricsSnapshot {
            active_connections: self.inner.active_connections.load(Ordering::Relaxed),
            accepted_connections: self.inner.accepted_connections.load(Ordering::Relaxed),
            access_denied_connections: self.inner.access_denied_connections.load(Ordering::Relaxed),
            connection_limit_rejections: self
                .inner
                .connection_limit_rejections
                .load(Ordering::Relaxed),
        }
    }

    fn connection_counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.inner.active_connections)
    }
}

fn increment_saturating(counter: &AtomicUsize) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

/// RAII guard that decrements the active connection counter on drop.
///
/// Returned by [`TcpServerListener::accept_socket`] or alongside connection
/// halves from [`TcpServerListener::accept`].
/// Callers must hold this guard for the lifetime of the connection to ensure
/// the counter stays accurate.
#[derive(Debug)]
pub struct ConnectionGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

/// TCP server listener with access control and connection limits.
pub struct TcpServerListener {
    listener: TcpListener,
    config: TcpServerConfig,
    metrics: TcpServerMetrics,
}

impl TcpServerListener {
    /// Bind to the given address and start listening.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::Io` if the bind fails.
    pub async fn bind(addr: SocketAddr, config: TcpServerConfig) -> Result<Self, TransportError> {
        debug!(
            addr = %addr,
            max_connections = config.max_connections,
            "binding TCP Modbus listener"
        );
        let listener = TcpListener::bind(addr).await?;
        debug!(addr = %listener.local_addr()?, "TCP Modbus listener bound");
        Ok(Self {
            listener,
            config,
            metrics: TcpServerMetrics::new(),
        })
    }

    /// Accept the next incoming connection, returning split transport halves.
    ///
    /// Applies access control and connection limits before returning.
    /// Silently drops denied or over-limit connections and retries.
    ///
    /// The returned [`ConnectionGuard`] automatically decrements the active
    /// connection counter when dropped. Callers must hold it for the lifetime
    /// of the connection.
    ///
    /// # Errors
    ///
    /// - `TransportError::Io` on accept failure.
    pub async fn accept(
        &self,
    ) -> Result<(TcpSink, TcpRecvStream, SocketAddr, ConnectionGuard), TransportError> {
        let (stream, addr, guard) = self.accept_socket().await?;
        let framed = Framed::new(stream, MbapCodec);
        let (sink, recv_stream) = framed.split();
        Ok((
            TcpSink::new(sink, self.config.tcp.write_timeout),
            TcpRecvStream::new(recv_stream, self.config.tcp.read_timeout),
            addr,
            guard,
        ))
    }

    /// Accept an unframed TCP socket after IP ACL, connection reservation and
    /// `TCP_NODELAY` setup. Denied/over-limit sockets are dropped and retried.
    ///
    /// The socket is not encrypted or authenticated. An upgrading caller must
    /// retain the guard through handshake and session, and provide its own
    /// handshake cancellation/deadline and subsequent I/O timeouts.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::Io` on accept or socket setup failure. A setup
    /// failure releases its reservation before returning.
    pub async fn accept_socket(
        &self,
    ) -> Result<(TcpStream, SocketAddr, ConnectionGuard), TransportError> {
        loop {
            let (stream, addr) = self.listener.accept().await?;
            trace!(peer_addr = %addr, "accepted TCP connection");

            // Check access control.
            if let Some(ref ac) = self.config.access_control
                && !ac.is_allowed(&addr.ip())
            {
                self.metrics.record_access_denied();
                debug!(peer_addr = %addr, "dropping TCP connection denied by access control");
                continue;
            }

            let Some(guard) = self.metrics.reserve_connection(self.config.max_connections) else {
                debug!(
                    peer_addr = %addr,
                    active_connections = self.metrics.snapshot().active_connections,
                    max_connections = self.config.max_connections,
                    "dropping TCP connection over limit"
                );
                continue;
            };
            let active_connections = self.metrics.snapshot().active_connections;
            trace!(
                peer_addr = %addr,
                active_connections,
                "tracking accepted TCP connection"
            );

            // The guard already owns the reservation, so every setup error
            // releases capacity before it leaves this method.
            let (stream, guard) = finish_admission(guard, || {
                stream.set_nodelay(self.config.tcp.tcp_nodelay)?;
                Ok::<_, std::io::Error>(stream)
            })?;

            self.metrics.record_accepted();
            return Ok((stream, addr, guard));
        }
    }

    /// Returns the local address the listener is bound to.
    ///
    /// # Errors
    ///
    /// Returns `TransportError::Io` if the address cannot be determined.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        Ok(self.listener.local_addr()?)
    }

    /// Returns a handle to the active connection counter for decrementing on drop.
    #[must_use]
    pub fn connection_counter(&self) -> Arc<AtomicUsize> {
        self.metrics.connection_counter()
    }

    /// Returns a cloneable handle to listener counters.
    #[must_use]
    pub fn metrics(&self) -> TcpServerMetrics {
        self.metrics.clone()
    }
}

fn finish_admission<T, E>(
    guard: ConnectionGuard,
    configure: impl FnOnce() -> Result<T, E>,
) -> Result<(T, ConnectionGuard), E> {
    let configured = configure()?;
    Ok((configured, guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_failure_releases_reserved_connection() {
        let metrics = TcpServerMetrics::new();
        let guard = metrics.reserve_connection(1).unwrap();

        let result: Result<((), ConnectionGuard), ()> = finish_admission(guard, || Err(()));

        assert!(result.is_err());
        assert_eq!(metrics.snapshot().active_connections, 0);
        assert!(metrics.reserve_connection(1).is_some());
    }

    #[tokio::test]
    async fn raw_admission_preserves_limit_nodelay_and_guard_lifetime() {
        let listener = Arc::new(
            TcpServerListener::bind(
                "127.0.0.1:0".parse().unwrap(),
                TcpServerConfig {
                    max_connections: 1,
                    tcp: crate::config::TcpConfig {
                        tcp_nodelay: false,
                        ..crate::config::TcpConfig::default()
                    },
                    ..TcpServerConfig::default()
                },
            )
            .await
            .unwrap(),
        );
        let address = listener.local_addr().unwrap();
        let first = TcpStream::connect(address).await.unwrap();
        let (socket, _, guard) = listener.accept_socket().await.unwrap();
        assert!(!socket.nodelay().unwrap());
        drop(socket);
        assert_eq!(listener.metrics().snapshot().active_connections, 1);
        let accepting = Arc::clone(&listener);
        let task = tokio::spawn(async move { accepting.accept_socket().await });
        let rejected = TcpStream::connect(address).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while listener.metrics().snapshot().connection_limit_rejections == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(guard);
        let third = TcpStream::connect(address).await.unwrap();
        let (socket, _, guard) = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!socket.nodelay().unwrap());
        assert_eq!(listener.metrics().snapshot().accepted_connections, 2);
        drop((guard, socket, first, rejected, third));
        assert_eq!(listener.metrics().snapshot().active_connections, 0);
    }
}
