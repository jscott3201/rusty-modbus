//! Transport error types.

use rusty_modbus_frame::FrameError;
use rusty_modbus_types::FunctionCode;

/// Errors that can occur during transport operations.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Underlying I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Frame-level error (decode, encode, CRC, protocol ID).
    #[error("frame error: {0}")]
    Frame(#[from] FrameError),

    /// Operation timed out.
    #[error("operation timed out")]
    Timeout,

    /// Connection was closed by the remote peer.
    #[error("disconnected")]
    Disconnected,

    /// Connection denied by access control.
    #[error("access denied")]
    AccessDenied,

    /// TLS handshake failure (used by `modbus-tls`, defined here for `TransportError` unification).
    #[error("TLS handshake failed: {0}")]
    TlsHandshake(String),

    /// Authorization denied by server policy.
    #[error("authorization denied for role {role:?} on {function_code}")]
    AuthorizationDenied {
        /// Client role from certificate (if any).
        role: Option<String>,
        /// The function code that was denied.
        function_code: FunctionCode,
    },
}

/// TCP-specific error type (wraps `TransportError` for TCP context).
pub type TcpError = TransportError;
