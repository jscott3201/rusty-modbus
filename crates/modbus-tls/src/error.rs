//! TLS transport error types.

use modbus_frame::FrameError;

/// Errors that can occur during TLS transport operations.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// TLS handshake failure.
    #[error("TLS handshake failed: {0}")]
    Handshake(String),

    /// Certificate validation failure.
    #[error("certificate error: {0}")]
    Certificate(String),

    /// Authorization denied by server policy.
    #[error("authorization denied: {0}")]
    AuthorizationDenied(String),

    /// Underlying I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Frame-level error.
    #[error("frame error: {0}")]
    Frame(#[from] FrameError),

    /// Rustls error.
    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),

    /// Operation timed out.
    #[error("operation timed out")]
    Timeout,

    /// Connection was closed.
    #[error("disconnected")]
    Disconnected,
}

impl From<TlsError> for modbus_tcp::TransportError {
    fn from(e: TlsError) -> Self {
        match e {
            TlsError::Io(io) => Self::Io(io),
            TlsError::Frame(f) => Self::Frame(f),
            TlsError::Timeout => Self::Timeout,
            TlsError::Disconnected => Self::Disconnected,
            TlsError::Rustls(e) => Self::TlsHandshake(e.to_string()),
            TlsError::Handshake(msg)
            | TlsError::Certificate(msg)
            | TlsError::AuthorizationDenied(msg) => Self::TlsHandshake(msg),
        }
    }
}
