//! Frame-layer error types.

use rusty_modbus_codec::DecodeError;

/// Errors that can occur during frame-level decode/encode.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// MBAP protocol identifier is not 0x0000.
    #[error("invalid MBAP protocol identifier: {0:#06X}")]
    InvalidProtocolId(u16),

    /// MBAP length field exceeds maximum PDU size.
    #[error("MBAP length overflow: {0}")]
    LengthOverflow(u16),

    /// RTU CRC-16 mismatch.
    #[error("CRC mismatch: expected {expected:#06X}, got {actual:#06X}")]
    CrcMismatch {
        /// Expected CRC value.
        expected: u16,
        /// Actual CRC value from the frame.
        actual: u16,
    },

    /// Frame is truncated (not enough bytes).
    #[error("frame truncated")]
    Truncated,

    /// Underlying I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// PDU decode error.
    #[error("decode error: {0}")]
    Decode(#[from] DecodeError),
}
