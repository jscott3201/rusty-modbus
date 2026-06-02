//! TCP server listener — accepts incoming Modbus/TCP connections.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::StreamExt;
use rusty_modbus_frame::mbap::MbapCodec;
use tokio::net::TcpListener;
use tokio_util::codec::Framed;
use tracing::{debug, trace};

use crate::config::TcpServerConfig;
use crate::connect::{TcpRecvStream, TcpSink};
use crate::error::TransportError;

/// RAII guard that decrements the active connection counter on drop.
///
/// Returned alongside connection halves from [`TcpServerListener::accept`].
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
    active_connections: Arc<AtomicUsize>,
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
            active_connections: Arc::new(AtomicUsize::new(0)),
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
        loop {
            let (stream, addr) = self.listener.accept().await?;
            trace!(peer_addr = %addr, "accepted TCP connection");

            // Check access control.
            if let Some(ref ac) = self.config.access_control
                && !ac.is_allowed(&addr.ip())
            {
                debug!(peer_addr = %addr, "dropping TCP connection denied by access control");
                continue;
            }

            // Check connection limit.
            let current = self.active_connections.load(Ordering::Relaxed);
            if current >= self.config.max_connections {
                debug!(
                    peer_addr = %addr,
                    active_connections = current,
                    max_connections = self.config.max_connections,
                    "dropping TCP connection over limit"
                );
                continue;
            }
            self.active_connections.fetch_add(1, Ordering::Relaxed);
            trace!(
                peer_addr = %addr,
                active_connections = current + 1,
                "tracking accepted TCP connection"
            );

            // Configure socket.
            stream.set_nodelay(self.config.tcp.tcp_nodelay)?;

            let framed = Framed::new(stream, MbapCodec);
            let (sink, recv_stream) = framed.split();

            let sink = TcpSink::new(sink, self.config.tcp.write_timeout);
            let recv = TcpRecvStream::new(recv_stream, self.config.tcp.read_timeout);

            let guard = ConnectionGuard {
                counter: Arc::clone(&self.active_connections),
            };

            return Ok((sink, recv, addr, guard));
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
        Arc::clone(&self.active_connections)
    }
}
