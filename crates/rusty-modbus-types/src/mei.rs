//! Encapsulated Interface (MEI) types for FC 0x2B (Spec V1.1b3 §6.19–6.21).

/// MEI type codes for FC 0x2B.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum MeiType {
    /// `CANopen` General Reference (licensed to `CiA`).
    CanOpenGeneralReference = 0x0D,
    /// Read Device Identification.
    ReadDeviceIdentification = 0x0E,
}

impl MeiType {
    /// Parse from a raw byte. Returns `None` for unknown MEI types.
    #[must_use]
    pub fn from_raw(byte: u8) -> Option<Self> {
        match byte {
            0x0D => Some(Self::CanOpenGeneralReference),
            0x0E => Some(Self::ReadDeviceIdentification),
            _ => None,
        }
    }

    /// The raw `u8` wire value.
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// Device identification access code. Spec V1.1b3 §6.21.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum DeviceIdCode {
    /// Read basic identification (stream access).
    BasicStream = 0x01,
    /// Read regular identification (stream access).
    RegularStream = 0x02,
    /// Read extended identification (stream access).
    ExtendedStream = 0x03,
    /// Read one specific identification object (individual access).
    Individual = 0x04,
}

impl DeviceIdCode {
    /// Parse from a raw byte. Returns `None` for unknown codes.
    #[must_use]
    pub fn from_raw(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(Self::BasicStream),
            0x02 => Some(Self::RegularStream),
            0x03 => Some(Self::ExtendedStream),
            0x04 => Some(Self::Individual),
            _ => None,
        }
    }

    /// The raw `u8` wire value.
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// Standard device identification object IDs. Spec V1.1b3 §6.21.
///
/// Objects 0x00–0x02 are **mandatory** (Basic).
/// Objects 0x03–0x06 are optional (Regular).
/// Objects 0x80–0xFF are vendor-specific (Extended).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum DeviceIdObject {
    /// Vendor name (mandatory).
    VendorName = 0x00,
    /// Product code (mandatory).
    ProductCode = 0x01,
    /// Major/minor revision (mandatory).
    MajorMinorRevision = 0x02,
    /// Vendor URL (optional).
    VendorUrl = 0x03,
    /// Product name (optional).
    ProductName = 0x04,
    /// Model name (optional).
    ModelName = 0x05,
    /// User application name (optional).
    UserApplicationName = 0x06,
}

impl DeviceIdObject {
    /// Parse from a raw byte. Returns `None` for unknown object IDs.
    #[must_use]
    pub fn from_raw(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::VendorName),
            0x01 => Some(Self::ProductCode),
            0x02 => Some(Self::MajorMinorRevision),
            0x03 => Some(Self::VendorUrl),
            0x04 => Some(Self::ProductName),
            0x05 => Some(Self::ModelName),
            0x06 => Some(Self::UserApplicationName),
            _ => None,
        }
    }

    /// The raw `u8` wire value.
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mei_type_round_trip() {
        assert_eq!(
            MeiType::from_raw(0x0D),
            Some(MeiType::CanOpenGeneralReference)
        );
        assert_eq!(
            MeiType::from_raw(0x0E),
            Some(MeiType::ReadDeviceIdentification)
        );
        assert_eq!(MeiType::CanOpenGeneralReference.code(), 0x0D);
        assert_eq!(MeiType::ReadDeviceIdentification.code(), 0x0E);
    }

    #[test]
    fn mei_type_unknown() {
        assert!(MeiType::from_raw(0x00).is_none());
        assert!(MeiType::from_raw(0x0F).is_none());
    }

    #[test]
    fn device_id_code_round_trip() {
        let variants = [
            (DeviceIdCode::BasicStream, 0x01),
            (DeviceIdCode::RegularStream, 0x02),
            (DeviceIdCode::ExtendedStream, 0x03),
            (DeviceIdCode::Individual, 0x04),
        ];
        for (code, wire) in variants {
            assert_eq!(code.code(), wire);
            assert_eq!(DeviceIdCode::from_raw(wire), Some(code));
        }
    }

    #[test]
    fn device_id_code_unknown() {
        assert!(DeviceIdCode::from_raw(0x00).is_none());
        assert!(DeviceIdCode::from_raw(0x05).is_none());
    }

    #[test]
    fn device_id_object_round_trip() {
        let variants = [
            (DeviceIdObject::VendorName, 0x00),
            (DeviceIdObject::ProductCode, 0x01),
            (DeviceIdObject::MajorMinorRevision, 0x02),
            (DeviceIdObject::VendorUrl, 0x03),
            (DeviceIdObject::ProductName, 0x04),
            (DeviceIdObject::ModelName, 0x05),
            (DeviceIdObject::UserApplicationName, 0x06),
        ];
        for (obj, wire) in variants {
            assert_eq!(obj.code(), wire);
            assert_eq!(DeviceIdObject::from_raw(wire), Some(obj));
        }
    }

    #[test]
    fn device_id_object_unknown() {
        assert!(DeviceIdObject::from_raw(0x07).is_none());
        assert!(DeviceIdObject::from_raw(0x80).is_none());
    }
}
