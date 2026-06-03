//! Encapsulated Interface Transport request: FC 2B (MEI).

use rusty_modbus_types::{DeviceIdCode, FunctionCode, MeiType};

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

/// FC 0x2B — Encapsulated Interface Transport request.
///
/// The MEI type selects the transport variant (e.g. `CANopen` General Reference
/// or Read Device Identification). The remaining `data` is MEI-type-specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncapsulatedInterfaceRequest<'buf> {
    /// MEI type code.
    pub mei_type: MeiType,
    /// MEI-type-specific data.
    pub data: &'buf [u8],
}

impl<'buf> EncapsulatedInterfaceRequest<'buf> {
    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is empty.
    /// Returns [`DecodeError::UnknownMeiType`] if the MEI type byte is not recognized.
    /// Returns [`DecodeError::LengthMismatch`] if a known fixed-length MEI request has extra bytes.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.is_empty() {
            return Err(DecodeError::Truncated {
                expected: 1,
                actual: 0,
            });
        }
        let mei_type = MeiType::from_raw(data[0]).ok_or(DecodeError::UnknownMeiType(data[0]))?;
        let payload = &data[1..];
        if mei_type == MeiType::ReadDeviceIdentification {
            DecodeError::check_exact_len(payload, 2)?;
            DeviceIdCode::from_raw(payload[0])
                .ok_or(DecodeError::InvalidDeviceIdCode(payload[0]))?;
        }
        Ok(Self {
            mei_type,
            data: payload,
        })
    }
}

impl Encode for EncapsulatedInterfaceRequest<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::EncapsulatedInterfaceTransport.code();
        buf[1] = self.mei_type.code();
        buf[2..2 + self.data.len()].copy_from_slice(self.data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC(1) + mei_type(1) + data
        2 + self.data.len()
    }
}
