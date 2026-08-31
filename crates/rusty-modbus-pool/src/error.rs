//! Pool error types.

use rusty_modbus_tcp::TransportError;

/// Errors that can occur during connection pool operations.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// No connections available and pool is at capacity.
    #[error("pool exhausted: no connections available")]
    Exhausted,

    /// Failed to establish a connection.
    #[error("connection failed: {0}")]
    ConnectionFailed(#[from] TransportError),

    /// The fixed pool-capacity acquisition deadline elapsed.
    #[error("pool capacity acquisition timed out")]
    Timeout,

    /// Pool is shutting down; no new connections will be issued.
    #[error("pool is shutting down")]
    ShuttingDown,
}

/// Errors that prevent a raw pool lease from becoming a reusable client session.
#[cfg(feature = "client")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReusableClientHandoffError {
    /// Direct access to either raw transport half permanently disqualified the lease.
    #[error("raw transport access disqualifies reusable client handoff")]
    RawTransportAccessed,

    /// The lease transport was already retired and is no longer available.
    #[error("pooled connection is no longer available for reusable client handoff")]
    LeaseUnavailable,
}
