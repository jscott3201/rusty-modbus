//! RAII wrapper for pooled connections.

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::time::Instant;

use rusty_modbus_tcp::{TcpRecvStream, TcpSink};

use crate::pool::{PoolEntry, PoolInner};

/// A connection checked out from the pool.
///
/// Automatically returns the connection to the pool when dropped.
/// Access the transport halves via [`sink()`](Self::sink) and [`stream()`](Self::stream).
pub struct PooledConnection {
    entry: Option<PoolEntry>,
    pool: Arc<Mutex<PoolInner>>,
}

impl PooledConnection {
    pub(crate) fn new(entry: PoolEntry, pool: Arc<Mutex<PoolInner>>) -> Self {
        Self {
            entry: Some(entry),
            pool,
        }
    }

    /// Mutable access to the write half of the transport.
    ///
    /// # Panics
    ///
    /// Panics if the connection has already been dropped.
    pub fn sink(&mut self) -> &mut TcpSink {
        &mut self
            .entry
            .as_mut()
            .expect("connection already returned")
            .sink
    }

    /// Mutable access to the read half of the transport.
    ///
    /// # Panics
    ///
    /// Panics if the connection has already been dropped.
    pub fn stream(&mut self) -> &mut TcpRecvStream {
        &mut self
            .entry
            .as_mut()
            .expect("connection already returned")
            .stream
    }

    /// The remote address this connection is connected to.
    ///
    /// # Panics
    ///
    /// Panics if the connection has already been dropped.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.entry
            .as_ref()
            .expect("connection already returned")
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
            .finish_non_exhaustive()
    }
}
