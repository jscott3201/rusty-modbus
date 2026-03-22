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

    /// Operation timed out.
    #[error("operation timed out")]
    Timeout,

    /// Pool is shutting down; no new connections will be issued.
    #[error("pool is shutting down")]
    ShuttingDown,
}
