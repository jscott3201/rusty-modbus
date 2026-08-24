//! Server error types.

use rusty_modbus_tcp::TransportError;

/// Invalid server limits rejected before binding a listen socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ServerConfigError {
    /// A zero connection limit cannot admit any connection.
    #[error("max_connections must be greater than zero")]
    ZeroMaxConnections,
    /// A zero transaction limit cannot describe valid request capacity.
    #[error("max_transactions must be greater than zero")]
    ZeroMaxTransactions,
    /// A zero timeout cannot provide a drain interval.
    #[error("shutdown_timeout must be nonzero")]
    ZeroShutdownTimeout,
}

/// Errors that can occur during server operations.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Configuration is invalid and no bind was attempted.
    #[error("invalid server configuration: {0}")]
    InvalidConfig(#[from] ServerConfigError),

    /// Transport-level error (I/O, framing).
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    /// Failed to bind the listen address.
    #[error("bind failed: {0}")]
    Bind(std::io::Error),

    /// Server is already running.
    #[error("server is already running")]
    AlreadyRunning,
}
