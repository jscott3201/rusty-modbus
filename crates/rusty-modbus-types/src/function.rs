//! Modbus function codes from Spec V1.1b3 §5.1.

/// Public function codes defined by the Modbus Application Protocol.
///
/// Every public function code from Spec V1.1b3 §5.1 is represented as a named
/// variant. Non-standard / vendor-specific codes are represented as
/// [`Custom`](Self::Custom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FunctionCode {
    /// Read coils (bit access, read-write). §6.1.
    ReadCoils,
    /// Read discrete inputs (bit access, read-only). §6.2.
    ReadDiscreteInputs,
    /// Read holding registers (16-bit, read-write). §6.3.
    ReadHoldingRegisters,
    /// Read input registers (16-bit, read-only). §6.4.
    ReadInputRegisters,
    /// Write single coil. §6.5.
    WriteSingleCoil,
    /// Write single register. §6.6.
    WriteSingleRegister,
    /// Read exception status (serial diagnostics). §6.7.
    ReadExceptionStatus,
    /// Diagnostics (serial diagnostics). §6.8.
    Diagnostics,
    /// Get comm event counter (serial diagnostics). §6.9.
    GetCommEventCounter,
    /// Get comm event log (serial diagnostics). §6.10.
    GetCommEventLog,
    /// Write multiple coils. §6.11.
    WriteMultipleCoils,
    /// Write multiple registers. §6.12.
    WriteMultipleRegisters,
    /// Report server ID (serial diagnostics). §6.13.
    ReportServerId,
    /// Read file record. §6.14.
    ReadFileRecord,
    /// Write file record. §6.15.
    WriteFileRecord,
    /// Mask write register. §6.16.
    MaskWriteRegister,
    /// Read/write multiple registers. §6.17.
    ReadWriteMultipleRegisters,
    /// Read FIFO queue. §6.18.
    ReadFifoQueue,
    /// Encapsulated interface transport (MEI). §6.19.
    EncapsulatedInterfaceTransport,
    /// Non-standard / vendor-specific nonzero function code.
    Custom(u8),
}

impl FunctionCode {
    /// Parse from a raw byte. Returns `None` for invalid code 0x00 and
    /// exception-flagged bytes (0x80+).
    ///
    /// Unknown nonzero, non-exception codes become [`Custom`](Self::Custom).
    /// Use [`from_exception_raw`](Self::from_exception_raw) for exception-flagged bytes.
    #[must_use]
    pub fn from_raw(byte: u8) -> Option<Self> {
        if byte == 0 || byte & 0x80 != 0 {
            return None;
        }
        Some(Self::from_byte(byte))
    }

    /// Parse from an exception-flagged byte. Strips the 0x80 flag before matching.
    ///
    /// Always succeeds — unknown codes become [`Custom`](Self::Custom).
    #[must_use]
    pub fn from_exception_raw(byte: u8) -> Self {
        Self::from_byte(byte & 0x7F)
    }

    /// Returns `true` if the byte has the exception flag set (MSB = 1).
    #[must_use]
    pub fn is_exception_response(byte: u8) -> bool {
        byte & 0x80 != 0
    }

    /// The raw `u8` wire value.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::ReadCoils => 0x01,
            Self::ReadDiscreteInputs => 0x02,
            Self::ReadHoldingRegisters => 0x03,
            Self::ReadInputRegisters => 0x04,
            Self::WriteSingleCoil => 0x05,
            Self::WriteSingleRegister => 0x06,
            Self::ReadExceptionStatus => 0x07,
            Self::Diagnostics => 0x08,
            Self::GetCommEventCounter => 0x0B,
            Self::GetCommEventLog => 0x0C,
            Self::WriteMultipleCoils => 0x0F,
            Self::WriteMultipleRegisters => 0x10,
            Self::ReportServerId => 0x11,
            Self::ReadFileRecord => 0x14,
            Self::WriteFileRecord => 0x15,
            Self::MaskWriteRegister => 0x16,
            Self::ReadWriteMultipleRegisters => 0x17,
            Self::ReadFifoQueue => 0x18,
            Self::EncapsulatedInterfaceTransport => 0x2B,
            Self::Custom(v) => v,
        }
    }

    /// The exception-flagged wire value (`code | 0x80`).
    #[must_use]
    pub fn exception_code(self) -> u8 {
        self.code() | 0x80
    }

    fn from_byte(byte: u8) -> Self {
        match byte {
            0x01 => Self::ReadCoils,
            0x02 => Self::ReadDiscreteInputs,
            0x03 => Self::ReadHoldingRegisters,
            0x04 => Self::ReadInputRegisters,
            0x05 => Self::WriteSingleCoil,
            0x06 => Self::WriteSingleRegister,
            0x07 => Self::ReadExceptionStatus,
            0x08 => Self::Diagnostics,
            0x0B => Self::GetCommEventCounter,
            0x0C => Self::GetCommEventLog,
            0x0F => Self::WriteMultipleCoils,
            0x10 => Self::WriteMultipleRegisters,
            0x11 => Self::ReportServerId,
            0x14 => Self::ReadFileRecord,
            0x15 => Self::WriteFileRecord,
            0x16 => Self::MaskWriteRegister,
            0x17 => Self::ReadWriteMultipleRegisters,
            0x18 => Self::ReadFifoQueue,
            0x2B => Self::EncapsulatedInterfaceTransport,
            other => Self::Custom(other),
        }
    }
}

impl core::fmt::Display for FunctionCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ReadCoils => write!(f, "Read Coils (0x01)"),
            Self::ReadDiscreteInputs => write!(f, "Read Discrete Inputs (0x02)"),
            Self::ReadHoldingRegisters => write!(f, "Read Holding Registers (0x03)"),
            Self::ReadInputRegisters => write!(f, "Read Input Registers (0x04)"),
            Self::WriteSingleCoil => write!(f, "Write Single Coil (0x05)"),
            Self::WriteSingleRegister => write!(f, "Write Single Register (0x06)"),
            Self::ReadExceptionStatus => write!(f, "Read Exception Status (0x07)"),
            Self::Diagnostics => write!(f, "Diagnostics (0x08)"),
            Self::GetCommEventCounter => write!(f, "Get Comm Event Counter (0x0B)"),
            Self::GetCommEventLog => write!(f, "Get Comm Event Log (0x0C)"),
            Self::WriteMultipleCoils => write!(f, "Write Multiple Coils (0x0F)"),
            Self::WriteMultipleRegisters => write!(f, "Write Multiple Registers (0x10)"),
            Self::ReportServerId => write!(f, "Report Server ID (0x11)"),
            Self::ReadFileRecord => write!(f, "Read File Record (0x14)"),
            Self::WriteFileRecord => write!(f, "Write File Record (0x15)"),
            Self::MaskWriteRegister => write!(f, "Mask Write Register (0x16)"),
            Self::ReadWriteMultipleRegisters => {
                write!(f, "Read/Write Multiple Registers (0x17)")
            }
            Self::ReadFifoQueue => write!(f, "Read FIFO Queue (0x18)"),
            Self::EncapsulatedInterfaceTransport => {
                write!(f, "Encapsulated Interface Transport (0x2B)")
            }
            Self::Custom(v) => write!(f, "Custom Function Code (0x{v:02X})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;

    use super::*;

    const ALL_VARIANTS: &[(FunctionCode, u8)] = &[
        (FunctionCode::ReadCoils, 0x01),
        (FunctionCode::ReadDiscreteInputs, 0x02),
        (FunctionCode::ReadHoldingRegisters, 0x03),
        (FunctionCode::ReadInputRegisters, 0x04),
        (FunctionCode::WriteSingleCoil, 0x05),
        (FunctionCode::WriteSingleRegister, 0x06),
        (FunctionCode::ReadExceptionStatus, 0x07),
        (FunctionCode::Diagnostics, 0x08),
        (FunctionCode::GetCommEventCounter, 0x0B),
        (FunctionCode::GetCommEventLog, 0x0C),
        (FunctionCode::WriteMultipleCoils, 0x0F),
        (FunctionCode::WriteMultipleRegisters, 0x10),
        (FunctionCode::ReportServerId, 0x11),
        (FunctionCode::ReadFileRecord, 0x14),
        (FunctionCode::WriteFileRecord, 0x15),
        (FunctionCode::MaskWriteRegister, 0x16),
        (FunctionCode::ReadWriteMultipleRegisters, 0x17),
        (FunctionCode::ReadFifoQueue, 0x18),
        (FunctionCode::EncapsulatedInterfaceTransport, 0x2B),
    ];

    #[test]
    fn round_trip_all_variants() {
        for &(fc, wire) in ALL_VARIANTS {
            assert_eq!(fc.code(), wire, "{fc:?} code mismatch");
            assert_eq!(
                FunctionCode::from_raw(wire),
                Some(fc),
                "from_raw({wire:#04X}) failed"
            );
        }
    }

    #[test]
    fn from_raw_rejects_exception_flagged() {
        for &(_, wire) in ALL_VARIANTS {
            assert!(
                FunctionCode::from_raw(wire | 0x80).is_none(),
                "from_raw should reject {:#04X}",
                wire | 0x80
            );
        }
    }

    #[test]
    fn from_exception_raw_strips_flag() {
        for &(fc, wire) in ALL_VARIANTS {
            assert_eq!(
                FunctionCode::from_exception_raw(wire | 0x80),
                fc,
                "from_exception_raw({:#04X}) failed",
                wire | 0x80
            );
        }
    }

    #[test]
    fn from_exception_raw_also_works_without_flag() {
        for &(fc, wire) in ALL_VARIANTS {
            assert_eq!(FunctionCode::from_exception_raw(wire), fc);
        }
    }

    #[test]
    fn from_exception_raw_custom() {
        assert_eq!(
            FunctionCode::from_exception_raw(0xC1),
            FunctionCode::Custom(0x41)
        );
    }

    #[test]
    fn exception_code_sets_high_bit() {
        for &(fc, wire) in ALL_VARIANTS {
            assert_eq!(fc.exception_code(), wire | 0x80);
        }
    }

    #[test]
    fn is_exception_response_checks_high_bit() {
        assert!(!FunctionCode::is_exception_response(0x03));
        assert!(FunctionCode::is_exception_response(0x83));
        assert!(!FunctionCode::is_exception_response(0x00));
        assert!(FunctionCode::is_exception_response(0x80));
    }

    #[test]
    fn unknown_non_exception_codes_return_custom() {
        assert_eq!(
            FunctionCode::from_raw(0x09),
            Some(FunctionCode::Custom(0x09))
        );
        assert_eq!(
            FunctionCode::from_raw(0x0A),
            Some(FunctionCode::Custom(0x0A))
        );
        assert_eq!(
            FunctionCode::from_raw(0x0D),
            Some(FunctionCode::Custom(0x0D))
        );
        // Function code 0x00 is invalid per Modbus Application Protocol §4.1.
        assert!(FunctionCode::from_raw(0x00).is_none());
        // Exception-flagged bytes still return None
        assert!(FunctionCode::from_raw(0xFF).is_none());
    }

    #[test]
    fn custom_round_trip() {
        let fc = FunctionCode::Custom(0x41);
        assert_eq!(fc.code(), 0x41);
        assert_eq!(FunctionCode::from_raw(0x41), Some(fc));
    }

    #[test]
    fn display_produces_readable_output() {
        let s = format!("{}", FunctionCode::ReadHoldingRegisters);
        assert!(s.contains("Read Holding Registers"));
        assert!(s.contains("0x03"));

        let s = format!("{}", FunctionCode::Custom(0x41));
        assert!(s.contains("Custom Function Code"));
        assert!(s.contains("0x41"));
    }

    #[test]
    fn variant_count_is_19() {
        assert_eq!(ALL_VARIANTS.len(), 19);
    }
}
