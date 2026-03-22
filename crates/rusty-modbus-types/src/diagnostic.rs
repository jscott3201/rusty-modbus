//! Diagnostic sub-function codes for FC 08 (Spec V1.1b3 §6.8.1).
//!
//! Serial-line diagnostics only.

/// Sub-function codes for the Diagnostics function code (FC 0x08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u16)]
pub enum DiagnosticSubFunction {
    /// Loopback test (0x0000).
    ReturnQueryData = 0x0000,
    /// Restart and set error mode (0x0001).
    RestartCommunicationsOption = 0x0001,
    /// Return diagnostic register (0x0002).
    ReturnDiagnosticRegister = 0x0002,
    /// Set ASCII mode delimiter (0x0003).
    ChangeAsciiInputDelimiter = 0x0003,
    /// Enter listen-only mode (0x0004).
    ForceListenOnlyMode = 0x0004,
    /// Reset all counters (0x000A).
    ClearCountersAndDiagRegister = 0x000A,
    /// Messages detected on bus (0x000B).
    ReturnBusMessageCount = 0x000B,
    /// CRC errors detected (0x000C).
    ReturnBusCommunicationErrorCount = 0x000C,
    /// Exception responses sent (0x000D).
    ReturnBusExceptionErrorCount = 0x000D,
    /// Messages addressed to this server (0x000E).
    ReturnServerMessageCount = 0x000E,
    /// Requests with no response (0x000F).
    ReturnServerNoResponseCount = 0x000F,
    /// NAK exception responses (0x0010).
    ReturnServerNakCount = 0x0010,
    /// Busy exception responses (0x0011).
    ReturnServerBusyCount = 0x0011,
    /// Character overrun errors (0x0012).
    ReturnBusCharacterOverrunCount = 0x0012,
    /// Clear overrun counter (0x0014).
    ClearOverrunCounterAndFlag = 0x0014,
}

impl DiagnosticSubFunction {
    /// Parse from a raw `u16` sub-function code. Returns `None` for unknown codes.
    #[must_use]
    pub fn from_raw(value: u16) -> Option<Self> {
        match value {
            0x0000 => Some(Self::ReturnQueryData),
            0x0001 => Some(Self::RestartCommunicationsOption),
            0x0002 => Some(Self::ReturnDiagnosticRegister),
            0x0003 => Some(Self::ChangeAsciiInputDelimiter),
            0x0004 => Some(Self::ForceListenOnlyMode),
            0x000A => Some(Self::ClearCountersAndDiagRegister),
            0x000B => Some(Self::ReturnBusMessageCount),
            0x000C => Some(Self::ReturnBusCommunicationErrorCount),
            0x000D => Some(Self::ReturnBusExceptionErrorCount),
            0x000E => Some(Self::ReturnServerMessageCount),
            0x000F => Some(Self::ReturnServerNoResponseCount),
            0x0010 => Some(Self::ReturnServerNakCount),
            0x0011 => Some(Self::ReturnServerBusyCount),
            0x0012 => Some(Self::ReturnBusCharacterOverrunCount),
            0x0014 => Some(Self::ClearOverrunCounterAndFlag),
            _ => None,
        }
    }

    /// The raw `u16` wire value.
    #[must_use]
    pub fn code(self) -> u16 {
        self as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_VARIANTS: &[(DiagnosticSubFunction, u16)] = &[
        (DiagnosticSubFunction::ReturnQueryData, 0x0000),
        (DiagnosticSubFunction::RestartCommunicationsOption, 0x0001),
        (DiagnosticSubFunction::ReturnDiagnosticRegister, 0x0002),
        (DiagnosticSubFunction::ChangeAsciiInputDelimiter, 0x0003),
        (DiagnosticSubFunction::ForceListenOnlyMode, 0x0004),
        (DiagnosticSubFunction::ClearCountersAndDiagRegister, 0x000A),
        (DiagnosticSubFunction::ReturnBusMessageCount, 0x000B),
        (DiagnosticSubFunction::ReturnBusCommunicationErrorCount, 0x000C),
        (DiagnosticSubFunction::ReturnBusExceptionErrorCount, 0x000D),
        (DiagnosticSubFunction::ReturnServerMessageCount, 0x000E),
        (DiagnosticSubFunction::ReturnServerNoResponseCount, 0x000F),
        (DiagnosticSubFunction::ReturnServerNakCount, 0x0010),
        (DiagnosticSubFunction::ReturnServerBusyCount, 0x0011),
        (DiagnosticSubFunction::ReturnBusCharacterOverrunCount, 0x0012),
        (DiagnosticSubFunction::ClearOverrunCounterAndFlag, 0x0014),
    ];

    #[test]
    fn round_trip_all_variants() {
        for &(dsf, wire) in ALL_VARIANTS {
            assert_eq!(dsf.code(), wire, "{dsf:?} code mismatch");
            assert_eq!(
                DiagnosticSubFunction::from_raw(wire),
                Some(dsf),
                "from_raw({wire:#06X}) failed"
            );
        }
    }

    #[test]
    fn unknown_codes_return_none() {
        assert!(DiagnosticSubFunction::from_raw(0x0005).is_none());
        assert!(DiagnosticSubFunction::from_raw(0x0009).is_none());
        assert!(DiagnosticSubFunction::from_raw(0x0013).is_none());
        assert!(DiagnosticSubFunction::from_raw(0xFFFF).is_none());
    }

    #[test]
    fn variant_count_is_15() {
        assert_eq!(ALL_VARIANTS.len(), 15);
    }
}
