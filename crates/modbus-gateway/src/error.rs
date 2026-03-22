//! Gateway error types.

use modbus_tcp::TransportError;

/// Errors that can occur during gateway operations.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// Transport-level error.
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    /// Failed to bind the listen address.
    #[error("bind failed: {0}")]
    Bind(std::io::Error),

    /// No route configured for this unit ID.
    #[error("no route for unit ID {0}")]
    NoRoute(u8),

    /// Serial device timed out.
    #[error("serial device timeout for unit ID {0}")]
    DeviceTimeout(u8),
}
