//! Server error types.

use modbus_tcp::TransportError;

/// Errors that can occur during server operations.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
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
