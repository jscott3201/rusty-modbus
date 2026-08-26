//! RAII wrapper for pooled connections.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::time::Instant;

use rusty_modbus_tcp::{TcpRecvStream, TcpSink};

use crate::pool::{PoolEntry, PoolInner};

/// Caller-supplied classification for why a checked-out connection was retired.
///
/// The pool does not automatically infer these reasons or treat them as proof of
/// connection health. Callers may use
/// [`suggested_for_transport_error`](Self::suggested_for_transport_error) for an
/// optional conservative suggestion, but only an explicit call to
/// [`PooledConnection::invalidate`] records the first classification.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeaseInvalidationReason {
    /// The caller chose to retire the connection without a narrower classification.
    CallerDirected,
    /// The caller observed cancellation that made safe connection reuse uncertain.
    Cancelled,
    /// The caller observed a timeout that made safe connection reuse uncertain.
    Timeout,
    /// The caller observed transport behavior that made safe reuse uncertain.
    Transport,
    /// The caller observed protocol behavior that made safe reuse uncertain.
    Protocol,
}

impl LeaseInvalidationReason {
    /// Suggests a conservative reason for a raw TCP transport error.
    ///
    /// `Some(reason)` means a direct caller should consider calling
    /// [`PooledConnection::invalidate`] only when `error` came from an operation
    /// on that currently held lease. This pure helper never invalidates the
    /// lease, mutates pool state, or proves the connection unhealthy.
    ///
    /// `None` means that no recommendation can be made from that error variant;
    /// it does not prove that the lease is healthy, synchronized, or safe to
    /// reuse. Access and authorization denials describe policy rather than raw
    /// lease synchronization. A TLS handshake failure is not produced by an
    /// established pool-owned raw TCP lease.
    ///
    /// Frame errors are deliberately treated conservatively because
    /// [`rusty_modbus_tcp::TransportError`] erases their direction. A local
    /// outbound encode validation can fail before I/O, so this suggestion may
    /// over-retire a lease but remains safe. Cancellation may produce no
    /// transport error; callers must explicitly use [`Self::Cancelled`] when it
    /// makes reuse uncertain.
    ///
    /// Every current transport error variant is matched explicitly. Adding a
    /// variant therefore makes this function fail to compile until its behavior
    /// is deliberately chosen.
    #[must_use]
    pub fn suggested_for_transport_error(error: &rusty_modbus_tcp::TransportError) -> Option<Self> {
        match error {
            rusty_modbus_tcp::TransportError::Io(_)
            | rusty_modbus_tcp::TransportError::Disconnected => Some(Self::Transport),
            rusty_modbus_tcp::TransportError::Timeout => Some(Self::Timeout),
            rusty_modbus_tcp::TransportError::Frame(_) => Some(Self::Protocol),
            rusty_modbus_tcp::TransportError::AccessDenied
            | rusty_modbus_tcp::TransportError::TlsHandshake(_)
            | rusty_modbus_tcp::TransportError::AuthorizationDenied { .. } => None,
        }
    }
}

/// A connection checked out from the pool.
///
/// A healthy connection automatically returns to the pool when dropped. Call
/// [`invalidate`](Self::invalidate) to retire its transport halves immediately
/// instead. Access the transport halves via [`sink()`](Self::sink) and
/// [`stream()`](Self::stream).
pub struct PooledConnection {
    entry: Option<PoolEntry>,
    pool: Arc<Mutex<PoolInner>>,
    invalidation_reason: Option<LeaseInvalidationReason>,
}

impl PooledConnection {
    pub(crate) fn new(entry: PoolEntry, pool: Arc<Mutex<PoolInner>>) -> Self {
        Self {
            entry: Some(entry),
            pool,
            invalidation_reason: None,
        }
    }

    /// Immediately retire this connection instead of returning it to the pool.
    ///
    /// The first call releases the lease's active pool capacity and drops its TCP
    /// transport halves without placing the connection in the idle pool. The
    /// first `reason` is sticky; later calls are no-ops.
    ///
    /// Callers must invalidate after a timeout or cancellation when I/O may have
    /// occurred, or whenever transport, framing, or protocol behavior leaves
    /// stream synchronization ambiguous and the lease must not be reused.
    ///
    /// Reasons are caller classifications only. The pool does not automatically
    /// detect transport or protocol health, map errors to reasons, invalidate a
    /// lease, probe liveness, or prove stream synchronization. Callers may opt
    /// into [`LeaseInvalidationReason::suggested_for_transport_error`] and then
    /// decide whether to invalidate the currently held lease.
    pub fn invalidate(&mut self, reason: LeaseInvalidationReason) {
        if self.invalidation_reason.is_some() {
            return;
        }

        let Some(entry) = self.entry.take() else {
            return;
        };
        self.invalidation_reason = Some(reason);

        let is_priority = entry.is_priority;
        let addr = entry.addr;
        {
            let mut inner = self.pool.lock();
            inner.release_active(is_priority, addr);
        }

        // Retire the TCP halves only after releasing the pool mutex.
        drop(entry);
    }

    /// The caller's first invalidation reason, or `None` if none was recorded.
    ///
    /// `None` does not prove that the lease is healthy or safe to reuse.
    #[must_use]
    pub fn invalidation_reason(&self) -> Option<LeaseInvalidationReason> {
        self.invalidation_reason
    }

    /// Mutable access to the write half of the transport.
    ///
    /// # Panics
    ///
    /// Panics if the connection has already been returned to the pool or invalidated.
    pub fn sink(&mut self) -> &mut TcpSink {
        &mut self
            .entry
            .as_mut()
            .expect("connection already returned or invalidated")
            .sink
    }

    /// Mutable access to the read half of the transport.
    ///
    /// # Panics
    ///
    /// Panics if the connection has already been returned to the pool or invalidated.
    pub fn stream(&mut self) -> &mut TcpRecvStream {
        &mut self
            .entry
            .as_mut()
            .expect("connection already returned or invalidated")
            .stream
    }

    /// The remote address this connection is connected to.
    ///
    /// # Panics
    ///
    /// Panics if the connection has already been returned to the pool or invalidated.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.entry
            .as_ref()
            .expect("connection already returned or invalidated")
            .addr
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        if let Some(mut entry) = self.entry.take() {
            entry.last_used = Instant::now();
            let mut inner = self.pool.lock();
            inner.release_active(entry.is_priority, entry.addr);
            if !inner.shutting_down {
                inner.idle.push(entry);
            }
        }
    }
}

impl std::fmt::Debug for PooledConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledConnection")
            .field("addr", &self.entry.as_ref().map(|e| e.addr))
            .field("invalidation_reason", &self.invalidation_reason)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use rusty_modbus_frame::FrameError;
    use rusty_modbus_tcp::TransportError;
    use rusty_modbus_types::FunctionCode;

    use super::LeaseInvalidationReason;

    fn suggestion(error: &TransportError) -> Option<LeaseInvalidationReason> {
        LeaseInvalidationReason::suggested_for_transport_error(error)
    }

    #[test]
    fn io_error_suggests_transport_invalidation() {
        let error = TransportError::Io(Error::new(ErrorKind::BrokenPipe, "test failure"));

        assert_eq!(suggestion(&error), Some(LeaseInvalidationReason::Transport));
    }

    #[test]
    fn timeout_suggests_timeout_invalidation() {
        assert_eq!(
            suggestion(&TransportError::Timeout),
            Some(LeaseInvalidationReason::Timeout)
        );
    }

    #[test]
    fn disconnected_suggests_transport_invalidation() {
        assert_eq!(
            suggestion(&TransportError::Disconnected),
            Some(LeaseInvalidationReason::Transport)
        );
    }

    #[test]
    fn frame_error_suggests_protocol_invalidation() {
        let error = TransportError::Frame(FrameError::InvalidProtocolId(1));

        assert_eq!(suggestion(&error), Some(LeaseInvalidationReason::Protocol));
    }

    #[test]
    fn access_denied_makes_no_recommendation() {
        assert_eq!(suggestion(&TransportError::AccessDenied), None);
    }

    #[test]
    fn tls_handshake_error_makes_no_recommendation() {
        let error = TransportError::TlsHandshake("test failure".to_owned());

        assert_eq!(suggestion(&error), None);
    }

    #[test]
    fn authorization_denied_makes_no_recommendation() {
        let error = TransportError::AuthorizationDenied {
            role: Some("operator".to_owned()),
            function_code: FunctionCode::WriteSingleRegister,
        };

        assert_eq!(suggestion(&error), None);
    }
}
