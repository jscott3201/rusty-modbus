//! Crate-level error types for type validation.

use crate::function::FunctionCode;

/// Errors that can occur during type-level validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeError {
    /// Unknown function code byte.
    UnknownFunctionCode(u8),
    /// Unknown exception code byte.
    UnknownExceptionCode(u8),
    /// Quantity out of valid range for the given function code.
    QuantityOutOfRange {
        /// The function code the quantity was validated against.
        function: FunctionCode,
        /// The invalid quantity value.
        quantity: u16,
    },
    /// Address + quantity would overflow the 16-bit address space.
    AddressOverflow {
        /// The starting address.
        address: u16,
        /// The quantity that causes overflow.
        quantity: u16,
    },
    /// MBAP protocol identifier is not 0x0000.
    InvalidProtocolId(u16),
    /// Coil wire value is neither 0xFF00 nor 0x0000.
    InvalidCoilValue(u16),
}

impl core::fmt::Display for TypeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownFunctionCode(byte) => {
                write!(f, "unknown function code: {byte:#04X}")
            }
            Self::UnknownExceptionCode(byte) => {
                write!(f, "unknown exception code: {byte:#04X}")
            }
            Self::QuantityOutOfRange { function, quantity } => {
                write!(f, "quantity {quantity} out of range for {function}")
            }
            Self::AddressOverflow { address, quantity } => {
                write!(
                    f,
                    "address {address} + quantity {quantity} overflows 16-bit address space"
                )
            }
            Self::InvalidProtocolId(id) => {
                write!(f, "invalid MBAP protocol identifier: {id:#06X}")
            }
            Self::InvalidCoilValue(value) => {
                write!(f, "invalid coil wire value: {value:#06X}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TypeError {}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::*;

    #[test]
    fn display_unknown_function_code() {
        let e = TypeError::UnknownFunctionCode(0x42);
        let s = format!("{e}");
        assert!(s.contains("unknown function code"));
        assert!(s.contains("0x42"));
    }

    #[test]
    fn display_unknown_exception_code() {
        let e = TypeError::UnknownExceptionCode(0x99);
        let s = format!("{e}");
        assert!(s.contains("unknown exception code"));
        assert!(s.contains("0x99"));
    }

    #[test]
    fn display_quantity_out_of_range() {
        let e = TypeError::QuantityOutOfRange {
            function: FunctionCode::ReadHoldingRegisters,
            quantity: 200,
        };
        let s = format!("{e}");
        assert!(s.contains("200"));
        assert!(s.contains("Read Holding Registers"));
    }

    #[test]
    fn display_address_overflow() {
        let e = TypeError::AddressOverflow {
            address: 65535,
            quantity: 1,
        };
        let s = format!("{e}");
        assert!(s.contains("65535"));
        assert!(s.contains("overflows"));
    }

    #[test]
    fn display_invalid_protocol_id() {
        let e = TypeError::InvalidProtocolId(0x1234);
        let s = format!("{e}");
        assert!(s.contains("protocol identifier"));
        assert!(s.contains("0x1234"));
    }

    #[test]
    fn display_invalid_coil_value() {
        let e = TypeError::InvalidCoilValue(0x0001);
        let s = format!("{e}");
        assert!(s.contains("coil wire value"));
    }
}
