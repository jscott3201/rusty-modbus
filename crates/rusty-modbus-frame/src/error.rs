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

    /// MBAP length field cannot contain the Unit Identifier plus a function code.
    #[error("invalid MBAP length: {declared} (minimum {minimum})")]
    InvalidLength {
        /// Declared MBAP length field value.
        declared: u16,
        /// Minimum valid MBAP length field value.
        minimum: u16,
    },

    /// MBAP length field does not match the encoded payload size.
    #[error("MBAP length mismatch: declared {declared}, actual {actual}")]
    LengthMismatch {
        /// Declared MBAP length field value.
        declared: u16,
        /// Actual length field value implied by the Unit Identifier and PDU bytes.
        actual: u16,
    },

    /// PDU payload is too short to contain the function code.
    #[error("invalid PDU length: {length} (minimum {minimum})")]
    InvalidPduLength {
        /// Actual PDU length in bytes.
        length: usize,
        /// Minimum valid PDU length in bytes.
        minimum: usize,
    },

    /// PDU payload exceeds the Modbus maximum size.
    #[error("PDU length overflow: {length} (maximum {maximum})")]
    PduLengthOverflow {
        /// Actual PDU length in bytes.
        length: usize,
        /// Maximum valid PDU length in bytes.
        maximum: usize,
    },

    /// RTU CRC-16 mismatch.
    #[error("CRC mismatch: expected {expected:#06X}, got {actual:#06X}")]
    CrcMismatch {
        /// Expected CRC value.
        expected: u16,
        /// Actual CRC value from the frame.
        actual: u16,
    },

    /// A strict RTU-over-TCP policy cannot derive a frame length for the function form.
    #[error("indeterminate strict RTU-over-TCP length for function code {function_code:#04X}")]
    IndeterminateRtuOverTcpFrameLength {
        /// Function code whose request or response grammar is not length-derivable.
        function_code: u8,
    },

    /// A declared strict RTU-over-TCP frame shape exceeds the RTU PDU bound.
    #[error("invalid strict RTU-over-TCP PDU length: {length} (maximum {maximum})")]
    InvalidRtuOverTcpFrameLength {
        /// PDU length implied by the frame's count fields.
        length: usize,
        /// Maximum legal PDU length.
        maximum: usize,
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
