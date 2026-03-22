//! Modbus exception codes from Spec V1.1b3 §7.

/// Exception codes returned in Modbus exception responses.
///
/// Every standard exception code from Spec V1.1b3 §7 and TCP Guide §4.4.2.5.
/// Non-standard codes from vendor devices are represented as [`Custom`](Self::Custom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ExceptionCode {
    /// Function code not supported (0x01).
    IllegalFunction,
    /// Address + quantity out of range (0x02).
    IllegalDataAddress,
    /// Malformed request data (0x03).
    IllegalDataValue,
    /// Unrecoverable error during processing (0x04).
    ServerDeviceFailure,
    /// Long-running operation accepted (0x05).
    Acknowledge,
    /// Server unable to accept request now (0x06).
    ServerDeviceBusy,
    /// Used with programming commands; request rejected (0x07).
    NegativeAcknowledge,
    /// File area consistency check failed (0x08).
    MemoryParityError,
    /// Gateway internal path error (0x0A).
    GatewayPathUnavailable,
    /// Target device did not respond (0x0B).
    GatewayTargetDeviceFailedToRespond,
    /// Non-standard exception code from a device.
    Custom(u8),
}

impl ExceptionCode {
    /// Parse from a raw byte.
    ///
    /// Known codes map to named variants; unknown codes become
    /// [`Custom`](Self::Custom).
    #[must_use]
    pub fn from_raw(byte: u8) -> Self {
        match byte {
            0x01 => Self::IllegalFunction,
            0x02 => Self::IllegalDataAddress,
            0x03 => Self::IllegalDataValue,
            0x04 => Self::ServerDeviceFailure,
            0x05 => Self::Acknowledge,
            0x06 => Self::ServerDeviceBusy,
            0x07 => Self::NegativeAcknowledge,
            0x08 => Self::MemoryParityError,
            0x0A => Self::GatewayPathUnavailable,
            0x0B => Self::GatewayTargetDeviceFailedToRespond,
            other => Self::Custom(other),
        }
    }

    /// The raw `u8` wire value.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::IllegalFunction => 0x01,
            Self::IllegalDataAddress => 0x02,
            Self::IllegalDataValue => 0x03,
            Self::ServerDeviceFailure => 0x04,
            Self::Acknowledge => 0x05,
            Self::ServerDeviceBusy => 0x06,
            Self::NegativeAcknowledge => 0x07,
            Self::MemoryParityError => 0x08,
            Self::GatewayPathUnavailable => 0x0A,
            Self::GatewayTargetDeviceFailedToRespond => 0x0B,
            Self::Custom(v) => v,
        }
    }
}

impl core::fmt::Display for ExceptionCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IllegalFunction => write!(f, "Illegal Function"),
            Self::IllegalDataAddress => write!(f, "Illegal Data Address"),
            Self::IllegalDataValue => write!(f, "Illegal Data Value"),
            Self::ServerDeviceFailure => write!(f, "Server Device Failure"),
            Self::Acknowledge => write!(f, "Acknowledge"),
            Self::ServerDeviceBusy => write!(f, "Server Device Busy"),
            Self::NegativeAcknowledge => write!(f, "Negative Acknowledge"),
            Self::MemoryParityError => write!(f, "Memory Parity Error"),
            Self::GatewayPathUnavailable => write!(f, "Gateway Path Unavailable"),
            Self::GatewayTargetDeviceFailedToRespond => {
                write!(f, "Gateway Target Device Failed to Respond")
            }
            Self::Custom(v) => write!(f, "Custom Exception (0x{v:02X})"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ExceptionCode {}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::*;

    const ALL_VARIANTS: &[(ExceptionCode, u8)] = &[
        (ExceptionCode::IllegalFunction, 0x01),
        (ExceptionCode::IllegalDataAddress, 0x02),
        (ExceptionCode::IllegalDataValue, 0x03),
        (ExceptionCode::ServerDeviceFailure, 0x04),
        (ExceptionCode::Acknowledge, 0x05),
        (ExceptionCode::ServerDeviceBusy, 0x06),
        (ExceptionCode::NegativeAcknowledge, 0x07),
        (ExceptionCode::MemoryParityError, 0x08),
        (ExceptionCode::GatewayPathUnavailable, 0x0A),
        (ExceptionCode::GatewayTargetDeviceFailedToRespond, 0x0B),
    ];

    #[test]
    fn round_trip_all_variants() {
        for &(ec, wire) in ALL_VARIANTS {
            assert_eq!(ec.code(), wire, "{ec:?} code mismatch");
            assert_eq!(
                ExceptionCode::from_raw(wire),
                ec,
                "from_raw({wire:#04X}) failed"
            );
        }
    }

    #[test]
    fn unknown_codes_return_custom() {
        assert_eq!(ExceptionCode::from_raw(0x00), ExceptionCode::Custom(0x00));
        assert_eq!(ExceptionCode::from_raw(0x09), ExceptionCode::Custom(0x09));
        assert_eq!(ExceptionCode::from_raw(0x0C), ExceptionCode::Custom(0x0C));
        assert_eq!(ExceptionCode::from_raw(0xFF), ExceptionCode::Custom(0xFF));
    }

    #[test]
    fn custom_round_trip() {
        let ec = ExceptionCode::Custom(0x42);
        assert_eq!(ec.code(), 0x42);
        assert_eq!(ExceptionCode::from_raw(0x42), ec);
    }

    #[test]
    fn display_produces_readable_output() {
        assert_eq!(
            format!("{}", ExceptionCode::IllegalDataAddress),
            "Illegal Data Address"
        );
        assert_eq!(
            format!("{}", ExceptionCode::GatewayTargetDeviceFailedToRespond),
            "Gateway Target Device Failed to Respond"
        );
        assert_eq!(
            format!("{}", ExceptionCode::NegativeAcknowledge),
            "Negative Acknowledge"
        );
        assert_eq!(
            format!("{}", ExceptionCode::Custom(0x42)),
            "Custom Exception (0x42)"
        );
    }

    #[test]
    fn variant_count_is_10() {
        assert_eq!(ALL_VARIANTS.len(), 10);
    }
}
