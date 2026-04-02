//! Coil value encoding helper.

/// Modbus coil value with special wire encoding (ON = 0xFF00, OFF = 0x0000).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CoilValue {
    /// Coil is energized.
    On,
    /// Coil is de-energized.
    Off,
}

impl CoilValue {
    /// Parse from the 16-bit wire representation.
    ///
    /// Returns `Some(On)` for 0xFF00, `Some(Off)` for 0x0000, `None` otherwise.
    #[must_use]
    pub fn from_wire(value: u16) -> Option<Self> {
        match value {
            0xFF00 => Some(Self::On),
            0x0000 => Some(Self::Off),
            _ => None,
        }
    }

    /// Convert to the 16-bit wire representation.
    #[must_use]
    pub fn to_wire(self) -> u16 {
        match self {
            Self::On => 0xFF00,
            Self::Off => 0x0000,
        }
    }

    /// Create from a boolean value.
    #[must_use]
    pub fn from_bool(b: bool) -> Self {
        if b { Self::On } else { Self::Off }
    }

    /// Convert to a boolean value.
    #[must_use]
    pub fn as_bool(self) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_wire_on() {
        assert_eq!(CoilValue::from_wire(0xFF00), Some(CoilValue::On));
    }

    #[test]
    fn from_wire_off() {
        assert_eq!(CoilValue::from_wire(0x0000), Some(CoilValue::Off));
    }

    #[test]
    fn from_wire_invalid() {
        assert!(CoilValue::from_wire(0x0001).is_none());
        assert!(CoilValue::from_wire(0xFF01).is_none());
        assert!(CoilValue::from_wire(0x00FF).is_none());
        assert!(CoilValue::from_wire(0x1234).is_none());
    }

    #[test]
    fn to_wire_round_trip() {
        assert_eq!(
            CoilValue::from_wire(CoilValue::On.to_wire()),
            Some(CoilValue::On)
        );
        assert_eq!(
            CoilValue::from_wire(CoilValue::Off.to_wire()),
            Some(CoilValue::Off)
        );
    }

    #[test]
    fn bool_conversion() {
        assert_eq!(CoilValue::from_bool(true), CoilValue::On);
        assert_eq!(CoilValue::from_bool(false), CoilValue::Off);
        assert!(CoilValue::On.as_bool());
        assert!(!CoilValue::Off.as_bool());
    }
}
