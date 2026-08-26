//! RAII wrapper for pooled connections.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::time::Instant;

use rusty_modbus_tcp::{TcpRecvStream, TcpSink};

use crate::pool::{PoolEntry, PoolInner};

/// Caller-supplied classification for why a checked-out connection was retired.
///
/// The pool does not infer these reasons, couple them to an error type, or treat
/// them as proof of connection health. They only record the first classification
/// supplied by the caller to [`PooledConnection::invalidate`].
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
    /// detect transport or protocol health, map errors to reasons, probe
    /// liveness, or prove stream synchronization.
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

    /// The caller's first invalidation reason, or `None` for a healthy lease.
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
