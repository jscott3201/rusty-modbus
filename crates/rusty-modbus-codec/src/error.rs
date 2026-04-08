//! Codec error types.

/// Errors that can occur when decoding a Modbus PDU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// PDU is shorter than the minimum for the function code.
    Truncated {
        /// Minimum expected length.
        expected: usize,
        /// Actual length received.
        actual: usize,
    },
    /// Function code byte is not recognized.
    UnknownFunctionCode(u8),
    /// Byte count field does not match actual remaining data length.
    ByteCountMismatch {
        /// Byte count declared in the PDU.
        declared: usize,
        /// Actual remaining data length.
        actual: usize,
    },
    /// Quantity value is outside the allowed range for this function code.
    QuantityOutOfRange {
        /// The invalid quantity value.
        quantity: u16,
    },
    /// Coil value is neither 0xFF00 nor 0x0000.
    InvalidCoilValue(u16),
    /// File sub-request reference type is not 6.
    InvalidReferenceType(u8),
    /// MEI type byte is not recognized.
    UnknownMeiType(u8),
    /// Exception code byte is not recognized.
    UnknownExceptionCode(u8),
    /// Diagnostic sub-function code is not recognized.
    UnknownDiagnosticSubFunction(u16),
    /// Device ID code byte is not recognized (FC 0x2B / MEI 0x0E).
    InvalidDeviceIdCode(u8),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { expected, actual } => {
                write!(f, "PDU truncated: expected {expected} bytes, got {actual}")
            }
            Self::UnknownFunctionCode(fc) => write!(f, "unknown function code: {fc:#04X}"),
            Self::ByteCountMismatch { declared, actual } => {
                write!(
                    f,
                    "byte count mismatch: declared {declared}, actual {actual}"
                )
            }
            Self::QuantityOutOfRange { quantity } => {
                write!(f, "quantity out of range: {quantity}")
            }
            Self::InvalidCoilValue(v) => write!(f, "invalid coil value: {v:#06X}"),
            Self::InvalidReferenceType(rt) => write!(f, "invalid reference type: {rt}"),
            Self::UnknownMeiType(mt) => write!(f, "unknown MEI type: {mt:#04X}"),
            Self::UnknownExceptionCode(ec) => write!(f, "unknown exception code: {ec:#04X}"),
            Self::UnknownDiagnosticSubFunction(sf) => {
                write!(f, "unknown diagnostic sub-function: {sf:#06X}")
            }
            Self::InvalidDeviceIdCode(code) => write!(f, "invalid device ID code: {code:#04X}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DecodeError {}

/// Errors that can occur when encoding a Modbus PDU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// Provided buffer is too small.
    BufferTooSmall {
        /// Required buffer size.
        required: usize,
        /// Available buffer size.
        available: usize,
    },
    /// Quantity exceeds protocol limits.
    QuantityOutOfRange {
        /// The invalid quantity value.
        quantity: u16,
    },
}

impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BufferTooSmall {
                required,
                available,
            } => {
                write!(
                    f,
                    "buffer too small: need {required} bytes, have {available}"
                )
            }
            Self::QuantityOutOfRange { quantity } => {
                write!(f, "quantity out of range: {quantity}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for EncodeError {}
