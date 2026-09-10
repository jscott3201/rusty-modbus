//! Bounded sequential, identity-only mutual-TLS serving.
//!
//! Every CA-authenticated peer may access the configured `DataStore`. This is
//! not role authorization or a complete Modbus Security serving profile.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::config::{AccessControl, TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::{ConnectionGuard, TcpServerListener};
use rusty_modbus_tls::{TlsError, TlsServerAcceptor, TlsServerConfig};
use rusty_modbus_types::UnitId;
use tokio::net::TcpStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::lifecycle::{ServerLifecycle, saturating_instant_add};
use crate::server::{
    AcceptBackoff, backoff_interrupted, drain_connections, handle_connection,
    report_connection_result,
};
use crate::{DataStore, DeviceIdentification, ServerMetrics, ShutdownOutcome};

/// Startup restrictions of the new identity-only serving profile.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityTlsConfigError {
    /// Identity-only serving still requires mutual TLS.
    #[error("identity-only TLS serving requires require_client_cert=true")]
    MutualTlsRequired,
    /// Callbacks require an authorizing runtime, not this profile.
    #[error(
        "authz_callback is unsupported by identity-only TLS serving; no callback may be configured"
    )]
    AuthorizationUnsupported,
    /// A required limit or configured timeout is zero.
    #[error("{0} must be nonzero")]
    ZeroLimit(&'static str),
    /// A capacity exceeds Tokio's representable semaphore bound.
    #[error("{0} exceeds the supported semaphore capacity")]
    CapacityTooLarge(&'static str),
}

/// Actionable startup errors; certificate and transport source errors are retained.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TlsServerStartError {
    /// Unsupported profile or unusable bounds, rejected before binding.
    #[error(transparent)]
    InvalidConfig(#[from] IdentityTlsConfigError),
    /// Certificate/key/CA configuration failed before binding.
    #[error(transparent)]
    Tls(#[from] TlsError),
    /// Listener bind or local-address lookup failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// Configuration containing only controls enforced by identity-only TLS serving.
///
/// No pipelining/transaction limit, outbound connect timeout, alternate port or
/// keepalive knob is accepted and ignored. Each connection is sequential.
#[derive(Debug, Clone)]
pub struct TlsModbusServerConfig {
    /// Explicit certificate configuration. mTLS is mandatory; callbacks are rejected.
    pub tls: TlsServerConfig,
    /// Bind address; constructor default is `0.0.0.0:802`.
    pub listen_addr: SocketAddr,
    /// Unit ID; default 1. Existing broadcast/direct-device behavior is retained.
    pub unit_id: UnitId,
    /// Bound on all admitted sockets, including handshakes; default 64.
    /// The effective bound is the minimum of this and `tls.max_connections`.
    pub max_connections: usize,
    /// Concurrent handshake cap; default 16, clamped to the effective connection bound.
    pub max_handshakes: usize,
    /// Absolute per-handshake budget from admission; default 5 seconds, nonzero.
    pub handshake_timeout: Duration,
    /// Graceful drain budget; default 10 seconds, nonzero.
    pub shutdown_timeout: Duration,
    /// Framed TLS receive timeout; default 30 seconds. `None` disables this timer.
    pub read_timeout: Option<Duration>,
    /// Framed TLS write timeout; default 30 seconds. `None` disables this timer.
    pub write_timeout: Option<Duration>,
    /// Applied before the TLS handshake; default true.
    pub tcp_nodelay: bool,
    /// IP ACL applied before handshake or framing; default unrestricted.
    pub access_control: Option<AccessControl>,
    /// Existing device identification responses.
    pub device_id: DeviceIdentification,
}

impl TlsModbusServerConfig {
    /// Choose the identity-only profile with explicit certificate configuration.
    #[must_use]
    pub fn new(tls: TlsServerConfig) -> Self {
        Self {
            tls,
            listen_addr: SocketAddr::from(([0, 0, 0, 0], 802)),
            unit_id: UnitId(1),
            max_connections: 64,
            max_handshakes: 16,
            handshake_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(10),
            read_timeout: Some(Duration::from_secs(30)),
            write_timeout: Some(Duration::from_secs(30)),
            tcp_nodelay: true,
            access_control: None,
            device_id: DeviceIdentification::default(),
        }
    }

    /// Validate all profile/limit choices before certificate loading or bind.
    pub fn validate(&self) -> Result<(), IdentityTlsConfigError> {
        if !self.tls.require_client_cert {
            return Err(IdentityTlsConfigError::MutualTlsRequired);
        }
        if self.tls.authz_callback.is_some() {
            return Err(IdentityTlsConfigError::AuthorizationUnsupported);
        }
        for (name, limit) in [
            ("max_connections", self.max_connections),
            ("tls.max_connections", self.tls.max_connections),
            ("max_handshakes", self.max_handshakes),
        ] {
            if limit == 0 {
                return Err(IdentityTlsConfigError::ZeroLimit(name));
            }
            if limit > Semaphore::MAX_PERMITS {
                return Err(IdentityTlsConfigError::CapacityTooLarge(name));
            }
        }
        for (name, timeout) in [
            ("handshake_timeout", Some(self.handshake_timeout)),
            ("shutdown_timeout", Some(self.shutdown_timeout)),
            ("read_timeout", self.read_timeout),
            ("write_timeout", self.write_timeout),
        ] {
            if timeout.is_some_and(|value| value.is_zero()) {
                return Err(IdentityTlsConfigError::ZeroLimit(name));
            }
        }
        Ok(())
    }
}

/// Nontransactional snapshot: TCP admission and TLS authentication are distinct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TlsServerMetrics {
    /// Existing counters. Accepted/active connections include unauthenticated sockets.
    pub server: ServerMetrics,
    /// Handshakes currently holding permits.
    pub active_handshakes: usize,
    /// Authenticated connections currently serving (not necessarily active requests).
    pub active_sessions: usize,
    /// Admitted handshake attempts.
    pub handshakes_started: usize,
    /// Successful TLS handshakes, not authorization decisions.
    pub handshakes_succeeded: usize,
    /// Failed TLS handshakes, including peer disconnect/protocol/certificate errors.
    pub handshakes_failed: usize,
    /// Attempts whose configured handshake deadline elapsed.
    pub handshakes_timed_out: usize,
    /// Attempts dropped/cancelled, including stop or server drop.
    pub handshakes_cancelled: usize,
    /// Admitted TCP sockets rejected without spawning because handshake slots were full.
    pub handshake_limit_rejections: usize,
}

#[derive(Default)]
struct TlsCounters {
    active_handshakes: AtomicUsize,
    active_sessions: AtomicUsize,
    started: AtomicUsize,
    succeeded: AtomicUsize,
    failed: AtomicUsize,
    timed_out: AtomicUsize,
    cancelled: AtomicUsize,
    rejected: AtomicUsize,
}

fn increment(counter: &AtomicUsize) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

struct HandshakeGuard {
    metrics: Arc<TlsCounters>,
    _permit: OwnedSemaphorePermit,
    finished: bool,
}

impl HandshakeGuard {
    fn new(metrics: Arc<TlsCounters>, permit: OwnedSemaphorePermit) -> Self {
        metrics.active_handshakes.fetch_add(1, Ordering::Relaxed);
        increment(&metrics.started);
        Self {
            metrics,
            _permit: permit,
            finished: false,
        }
    }

    fn finish(mut self, counter: &AtomicUsize) {
        increment(counter);
        self.finished = true;
    }
}

impl Drop for HandshakeGuard {
    fn drop(&mut self) {
        if !self.finished {
            increment(&self.metrics.cancelled);
        }
        self.metrics
            .active_handshakes
            .fetch_sub(1, Ordering::Relaxed);
    }
}

struct SessionGuard(Arc<TlsCounters>);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.0.active_sessions.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Serving<S: DataStore> {
    config: Arc<TlsModbusServerConfig>,
    store: Arc<S>,
    lifecycle: Arc<ServerLifecycle>,
    metrics: Arc<TlsCounters>,
    acceptor: TlsServerAcceptor,
    handshakes: Arc<Semaphore>,
}

/// Sequential identity-only mTLS server. Every trusted authenticated peer has
/// `DataStore` access; roles are not consulted for request authorization.
///
/// Stop uses the existing cancellation-safe coordinator. Drop requests abort
/// without waiting and does not promise graceful completion or immediate rebind.
pub struct TlsModbusServer<S: DataStore> {
    context: Arc<Serving<S>>,
    local_addr: SocketAddr,
}

impl<S: DataStore + 'static> TlsModbusServer<S> {
    /// Validate profile, load TLS material, bind, then start bounded supervision.
    pub async fn start(
        config: TlsModbusServerConfig,
        store: Arc<S>,
    ) -> Result<Self, TlsServerStartError> {
        config.validate()?;
        let acceptor = TlsServerAcceptor::new(&config.tls)?;
        let connection_limit = config.max_connections.min(config.tls.max_connections);
        let listener = TcpServerListener::bind(
            config.listen_addr,
            TcpServerConfig {
                max_connections: connection_limit,
                access_control: config.access_control.clone(),
                tcp: TcpConfig {
                    tcp_nodelay: config.tcp_nodelay,
                    ..TcpConfig::default()
                },
            },
        )
        .await?;
        let local_addr = listener.local_addr()?;
        let context = Arc::new(Serving {
            lifecycle: ServerLifecycle::new(listener.metrics()),
            handshakes: Arc::new(Semaphore::new(config.max_handshakes.min(connection_limit))),
            config: Arc::new(config),
            store,
            metrics: Arc::new(TlsCounters::default()),
            acceptor,
        });
        let task = tokio::spawn(supervise(listener, Arc::clone(&context)));
        context.lifecycle.install_supervisor(task);
        Ok(Self {
            context,
            local_addr,
        })
    }

    /// Seal admission, cancel handshakes, drain admitted requests, and join tasks.
    /// Concurrent/cancelled callers share the same absolute shutdown deadline.
    /// Forced abort is cooperative; non-yielding handlers can delay completion.
    pub async fn stop(&self) -> ShutdownOutcome {
        self.context
            .lifecycle
            .shutdown(self.context.config.shutdown_timeout)
            .await
    }

    /// Bound local address (including the assigned ephemeral port).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Configured data store, shared by all authenticated peers.
    #[must_use]
    pub fn store(&self) -> &S {
        self.context.store.as_ref()
    }

    /// Snapshot counters; fields can change during collection.
    #[must_use]
    pub fn metrics(&self) -> TlsServerMetrics {
        let metrics = &self.context.metrics;
        TlsServerMetrics {
            server: self.context.lifecycle.metrics(),
            active_handshakes: metrics.active_handshakes.load(Ordering::Relaxed),
            active_sessions: metrics.active_sessions.load(Ordering::Relaxed),
            handshakes_started: metrics.started.load(Ordering::Relaxed),
            handshakes_succeeded: metrics.succeeded.load(Ordering::Relaxed),
            handshakes_failed: metrics.failed.load(Ordering::Relaxed),
            handshakes_timed_out: metrics.timed_out.load(Ordering::Relaxed),
            handshakes_cancelled: metrics.cancelled.load(Ordering::Relaxed),
            handshake_limit_rejections: metrics.rejected.load(Ordering::Relaxed),
        }
    }
}

impl<S: DataStore> Drop for TlsModbusServer<S> {
    fn drop(&mut self) {
        self.context.lifecycle.abort_owned_tasks();
    }
}

async fn supervise<S: DataStore + 'static>(
    listener: TcpServerListener,
    context: Arc<Serving<S>>,
) -> ShutdownOutcome {
    let mut connections = JoinSet::new();
    let mut shutdown_rx = context.lifecycle.shutdown_receiver();
    let mut backoff = AcceptBackoff::new();
    let deadline = loop {
        if let Some(deadline) = context.lifecycle.shutdown_deadline() {
            break deadline;
        }
        tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {}
            joined = connections.join_next(), if !connections.is_empty() => {
                if let Some(result) = joined { report_connection_result(result); }
            }
            accepted = listener.accept_socket() => {
                if let Ok((socket, peer, connection)) = accepted {
                        backoff.reset();
                        if context.lifecycle.shutdown_deadline().is_some() { continue; }
                        let Ok(permit) = Arc::clone(&context.handshakes).try_acquire_owned() else {
                            increment(&context.metrics.rejected);
                            continue;
                        };
                        let handshake = HandshakeGuard::new(Arc::clone(&context.metrics), permit);
                        let deadline = saturating_instant_add(Instant::now(), context.config.handshake_timeout);
                        connections.spawn(serve_connection(socket, peer, connection, handshake, deadline, Arc::clone(&context)));
                } else {
                    context.lifecycle.metrics_handle().record_accept_error();
                    let _ = backoff_interrupted(backoff.failure_delay(), &mut shutdown_rx).await;
                }
            }
        }
    };
    drop(listener);
    drain_connections(&mut connections, deadline).await
}

async fn serve_connection<S: DataStore + 'static>(
    socket: TcpStream,
    peer: SocketAddr,
    connection: ConnectionGuard,
    handshake: HandshakeGuard,
    deadline: Instant,
    context: Arc<Serving<S>>,
) {
    let _connection = connection;
    let mut shutdown_rx = context.lifecycle.shutdown_receiver();
    if context.lifecycle.shutdown_deadline().is_some() {
        return;
    }
    let upgraded = tokio::select! {
        biased;
        _ = shutdown_rx.changed() => return,
        () = tokio::time::sleep_until(deadline) => {
            handshake.finish(&context.metrics.timed_out);
            return;
        }
        result = context.acceptor.accept(socket, context.config.read_timeout, context.config.write_timeout) => result,
    };
    let Ok((sink, stream, _role)) = upgraded else {
        handshake.finish(&context.metrics.failed);
        return;
    };
    handshake.finish(&context.metrics.succeeded);
    // Owned role metadata remains connection-local, never a request authorization verdict.
    context
        .metrics
        .active_sessions
        .fetch_add(1, Ordering::Relaxed);
    let _session = SessionGuard(Arc::clone(&context.metrics));
    handle_connection(
        sink,
        stream,
        peer,
        context.config.unit_id,
        Arc::clone(&context.store),
        context.config.device_id.clone(),
        Arc::clone(&context.lifecycle),
    )
    .await;
}
