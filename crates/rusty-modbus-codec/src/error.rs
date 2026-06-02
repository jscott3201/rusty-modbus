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
    /// PDU exceeds the Modbus maximum of 253 bytes.
    PduTooLarge {
        /// Actual PDU length.
        length: usize,
        /// Maximum allowed PDU length.
        maximum: usize,
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
    /// Byte count field is outside the allowed range for this function code.
    ByteCountOutOfRange {
        /// The invalid byte count.
        count: usize,
        /// The minimum allowed byte count.
        minimum: usize,
        /// The maximum allowed byte count.
        maximum: usize,
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
    /// Packed file-record data cannot be split into valid sub-record groups.
    InvalidFileRecordLength {
        /// Invalid packed file-record byte length.
        length: usize,
    },
    /// File-record address fields are outside the Modbus file-record model.
    FileRecordOutOfRange {
        /// File number.
        file_number: u16,
        /// Starting record number.
        record_number: u16,
        /// Number of records.
        record_length: u16,
    },
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
            Self::PduTooLarge { length, maximum } => {
                write!(f, "PDU too large: {length} bytes (maximum {maximum})")
            }
            Self::UnknownFunctionCode(fc) => write!(f, "unknown function code: {fc:#04X}"),
            Self::ByteCountMismatch { declared, actual } => {
                write!(
                    f,
                    "byte count mismatch: declared {declared}, actual {actual}"
                )
            }
            Self::ByteCountOutOfRange {
                count,
                minimum,
                maximum,
            } => write!(
                f,
                "byte count out of range: {count} (expected {minimum}..={maximum})"
            ),
            Self::QuantityOutOfRange { quantity } => {
                write!(f, "quantity out of range: {quantity}")
            }
            Self::InvalidCoilValue(v) => write!(f, "invalid coil value: {v:#06X}"),
            Self::InvalidReferenceType(rt) => write!(f, "invalid reference type: {rt}"),
            Self::InvalidFileRecordLength { length } => {
                write!(f, "invalid file-record data length: {length}")
            }
            Self::FileRecordOutOfRange {
                file_number,
                record_number,
                record_length,
            } => write!(
                f,
                "file record out of range: file {file_number}, record {record_number}, length {record_length}"
            ),
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
    /// Encoded PDU would exceed the Modbus maximum of 253 bytes.
    PduTooLarge {
        /// Encoded PDU length.
        length: usize,
        /// Maximum allowed PDU length.
        maximum: usize,
    },
    /// Quantity exceeds protocol limits.
    QuantityOutOfRange {
        /// The invalid quantity value.
        quantity: u16,
    },
    /// Declared byte count does not match the payload length.
    ByteCountMismatch {
        /// Declared byte count.
        declared: usize,
        /// Actual payload length.
        actual: usize,
    },
    /// Byte count is outside the allowed range for this function code.
    ByteCountOutOfRange {
        /// The invalid byte count.
        count: usize,
        /// The minimum allowed byte count.
        minimum: usize,
        /// The maximum allowed byte count.
        maximum: usize,
    },
    /// File sub-request reference type is not 6.
    InvalidReferenceType(u8),
    /// Packed file-record data cannot be split into valid sub-record groups.
    InvalidFileRecordLength {
        /// Invalid packed file-record byte length.
        length: usize,
    },
    /// File-record address fields are outside the Modbus file-record model.
    FileRecordOutOfRange {
        /// File number.
        file_number: u16,
        /// Starting record number.
        record_number: u16,
        /// Number of records.
        record_length: u16,
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
            Self::PduTooLarge { length, maximum } => {
                write!(f, "PDU too large: {length} bytes (maximum {maximum})")
            }
            Self::QuantityOutOfRange { quantity } => {
                write!(f, "quantity out of range: {quantity}")
            }
            Self::ByteCountMismatch { declared, actual } => {
                write!(
                    f,
                    "byte count mismatch: declared {declared}, actual {actual}"
                )
            }
            Self::ByteCountOutOfRange {
                count,
                minimum,
                maximum,
            } => write!(
                f,
                "byte count out of range: {count} (expected {minimum}..={maximum})"
            ),
            Self::InvalidReferenceType(rt) => write!(f, "invalid reference type: {rt}"),
            Self::InvalidFileRecordLength { length } => {
                write!(f, "invalid file-record data length: {length}")
            }
            Self::FileRecordOutOfRange {
                file_number,
                record_number,
                record_length,
            } => write!(
                f,
                "file record out of range: file {file_number}, record {record_number}, length {record_length}"
            ),
        }
    }
}

impl EncodeError {
    pub(crate) fn check_pdu_len(length: usize) -> Result<(), Self> {
        let maximum = rusty_modbus_types::MAX_PDU_SIZE;
        if length > maximum {
            Err(Self::PduTooLarge { length, maximum })
        } else {
            Ok(())
        }
    }

    pub(crate) fn check_quantity(quantity: u16, max: u16) -> Result<(), Self> {
        if quantity == 0 || quantity > max {
            Err(Self::QuantityOutOfRange { quantity })
        } else {
            Ok(())
        }
    }

    pub(crate) fn check_byte_count(declared: usize, actual: usize) -> Result<(), Self> {
        if declared == actual {
            Ok(())
        } else {
            Err(Self::ByteCountMismatch { declared, actual })
        }
    }

    pub(crate) fn check_byte_count_range(
        count: usize,
        minimum: usize,
        maximum: usize,
    ) -> Result<(), Self> {
        if (minimum..=maximum).contains(&count) {
            Ok(())
        } else {
            Err(Self::ByteCountOutOfRange {
                count,
                minimum,
                maximum,
            })
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for EncodeError {}
