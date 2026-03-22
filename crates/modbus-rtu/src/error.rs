//! RTU transport error types.

use modbus_frame::FrameError;
use modbus_tcp::TransportError;

/// Errors that can occur during RTU transport operations.
#[derive(Debug, thiserror::Error)]
pub enum RtuError {
    /// Underlying I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Frame-level error (decode, encode, CRC).
    #[error("frame error: {0}")]
    Frame(#[from] FrameError),

    /// Operation timed out.
    #[error("operation timed out")]
    Timeout,

    /// Connection was closed or serial port disconnected.
    #[error("disconnected")]
    Disconnected,

    /// Serial port configuration or access error.
    #[error("serial port error: {0}")]
    SerialPort(String),
}

impl From<RtuError> for TransportError {
    fn from(err: RtuError) -> Self {
        match err {
            RtuError::Io(e) => TransportError::Io(e),
            RtuError::Frame(e) => TransportError::Frame(e),
            RtuError::Timeout => TransportError::Timeout,
            RtuError::Disconnected => TransportError::Disconnected,
            RtuError::SerialPort(msg) => {
                TransportError::Io(std::io::Error::other(msg))
            }
        }
    }
}
