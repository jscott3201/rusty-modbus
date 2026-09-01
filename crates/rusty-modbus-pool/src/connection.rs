//! RAII wrapper for pooled connections.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Notify;

use rusty_modbus_tcp::{TcpRecvStream, TcpSink};

#[cfg(feature = "client")]
use rusty_modbus_client::{ClientConfig, ModbusClient};
#[cfg(feature = "client")]
use rusty_modbus_tcp::transport::TransportSink;

use crate::pool::{PoolEntry, PoolInner};

#[cfg(feature = "client")]
use crate::client_handoff::PooledClientSession;
#[cfg(feature = "client")]
use crate::error::ReusableClientHandoffError;

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
/// Dropping a raw lease always retires its exact transport and releases its
/// capacity charge; it never returns the connection to idle. With the `client`
/// feature, a pristine lease can instead enter the verdict-gated
/// [`PooledClientSession`] reuse path through
/// [`into_reusable_client`](Self::into_reusable_client). Calling
/// [`sink`](Self::sink) or [`stream`](Self::stream) permanently disqualifies that
/// lease from reusable handoff. [`addr`](Self::addr) does not.
pub struct PooledConnection {
    entry: Option<PoolEntry>,
    pool: Arc<Mutex<PoolInner>>,
    capacity_changed: Arc<Notify>,
    invalidation_reason: Option<LeaseInvalidationReason>,
    raw_accessed: bool,
}

impl PooledConnection {
    pub(crate) fn new(
        entry: PoolEntry,
        pool: Arc<Mutex<PoolInner>>,
        capacity_changed: Arc<Notify>,
    ) -> Self {
        Self {
            entry: Some(entry),
            pool,
            capacity_changed,
            invalidation_reason: None,
            raw_accessed: false,
        }
    }

    /// Hand this raw TCP lease to a high-level client that always retires it.
    ///
    /// This opt-in safety fence transfers both TCP halves and the active pool
    /// capacity charge to the returned [`ModbusClient`]. The charge is released
    /// exactly once, after both client-owned transport halves are gone. The TCP
    /// connection is never inserted into the idle pool, including after a
    /// healthy session or a concurrent pool shutdown, and capacity waiters are
    /// notified after retirement.
    ///
    /// Always retiring prevents a timed-out, cancelled, or delayed response from
    /// this client session from crossing a pool borrower boundary. It does not
    /// probe liveness, prove protocol synchronization, make a healthy client
    /// session reusable, or close the tracked F-017/F-018 recovery gaps.
    ///
    /// This method remains available after direct raw-half access because its
    /// transport can never return to idle. Prior raw access does not establish
    /// that starting a new operation is semantically safe.
    ///
    /// # Panics
    ///
    /// Panics if this lease was already invalidated, or if called without an
    /// active Tokio runtime in which to start the client-owned tasks. Unwinding
    /// either case still retires any transport and releases its active charge.
    #[cfg(feature = "client")]
    pub fn into_retiring_client(
        mut self,
        config: ClientConfig,
    ) -> ModbusClient<impl TransportSink + 'static> {
        let entry = self
            .entry
            .take()
            .expect("connection already returned or invalidated");
        crate::client_handoff::into_client(
            entry,
            Arc::clone(&self.pool),
            Arc::clone(&self.capacity_changed),
            config,
        )
    }

    /// Hand this raw TCP lease to an opt-in verdict-gated client session.
    ///
    /// The returned wrapper owns both TCP halves and the active pool capacity
    /// charge. Borrow its high-level client through [`PooledClientSession::client`]
    /// and consume it with [`PooledClientSession::shutdown_and_return`]. Only a
    /// completed graceful shutdown whose final local verdict is exactly
    /// [`rusty_modbus_client::SessionReuseVerdict::ReuseEligible`] can return the
    /// connection to idle. Drop, abort, cancellation, ambiguity, recovery failure,
    /// and pool shutdown all retire it instead.
    ///
    /// This local synchronization-safety contract assumes a conforming peer does
    /// not invent future duplicate traffic after all valid requests complete. It
    /// is not an active liveness probe or proof of permanent future silence, and
    /// does not close the tracked F-017/F-018 recovery gaps.
    ///
    /// Calling [`Self::sink`] or [`Self::stream`] before this method permanently
    /// disqualifies the lease. On rejection the transport retires and its active
    /// capacity charge is released exactly once. Calling [`Self::addr`] does not
    /// disqualify the handoff.
    ///
    /// # Errors
    ///
    /// Returns [`ReusableClientHandoffError::RawTransportAccessed`] after either
    /// raw half was accessed, or [`ReusableClientHandoffError::LeaseUnavailable`]
    /// if the lease was already retired.
    ///
    /// # Panics
    ///
    /// Panics if called without an active Tokio runtime in which to start the
    /// client-owned tasks. Unwinding still retires the transport and releases its
    /// active charge.
    #[cfg(feature = "client")]
    pub fn into_reusable_client(
        mut self,
        config: ClientConfig,
    ) -> Result<PooledClientSession, ReusableClientHandoffError> {
        if self.raw_accessed {
            let _ = self.retire_entry();
            return Err(ReusableClientHandoffError::RawTransportAccessed);
        }

        let Some(entry) = self.entry.take() else {
            return Err(ReusableClientHandoffError::LeaseUnavailable);
        };
        Ok(crate::client_handoff::into_reusable_session(
            entry,
            Arc::clone(&self.pool),
            Arc::clone(&self.capacity_changed),
            config,
        ))
    }

    /// Immediately retire this connection and record a caller classification.
    ///
    /// The first call releases the lease's active pool capacity and drops its TCP
    /// transport halves without placing the connection in the idle pool. The
    /// first `reason` is sticky; later calls are no-ops.
    ///
    /// Callers may invalidate after a timeout, cancellation, transport failure,
    /// or protocol ambiguity to classify the observation and release capacity
    /// before drop. Raw drop retires safely even without this classification.
    ///
    /// Reasons are caller classifications only. Default raw drop retirement does
    /// not invent an invalidation reason. The pool does not automatically classify
    /// a checked-out lease, probe liveness, or prove stream synchronization.
    /// Callers may opt into
    /// [`LeaseInvalidationReason::suggested_for_transport_error`] and then decide
    /// whether to invalidate the currently held lease.
    pub fn invalidate(&mut self, reason: LeaseInvalidationReason) {
        if self.invalidation_reason.is_some() {
            return;
        }

        if self.entry.is_none() {
            return;
        }
        self.invalidation_reason = Some(reason);
        let _ = self.retire_entry();
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
        self.raw_accessed = true;
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
        self.raw_accessed = true;
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

    /// Retire one exact checked-out entry and release its active charge.
    fn retire_entry(&mut self) -> Option<bool> {
        let entry = self.entry.take()?;
        let is_priority = entry.is_priority;
        let addr = entry.addr;
        {
            let mut inner = self.pool.lock();
            inner.retire_active(is_priority, addr);
        }

        // Retire the TCP halves only after releasing the pool mutex.
        drop(entry);
        self.capacity_changed.notify_waiters();
        Some(is_priority)
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        if let Some(is_priority) = self.retire_entry() {
            tracing::debug!(
                target: "rusty_modbus_pool::raw_lease",
                trigger = "drop",
                raw_accessed = self.raw_accessed,
                is_priority,
                "pooled_raw_connection_retired"
            );
        }
    }
}

impl std::fmt::Debug for PooledConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledConnection")
            .field("addr", &self.entry.as_ref().map(|e| e.addr))
            .field("invalidation_reason", &self.invalidation_reason)
            .field("raw_accessed", &self.raw_accessed)
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
