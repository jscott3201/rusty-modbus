//! Response type for Encapsulated Interface Transport (FC 2B).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::{FunctionCode, MeiType};

/// Response to an Encapsulated Interface Transport request (FC 0x2B).
#[derive(Debug)]
pub struct EncapsulatedInterfaceResponse<'buf> {
    /// The MEI type code.
    pub mei_type: MeiType,
    /// MEI-specific data bytes.
    pub data: &'buf [u8],
}

impl<'buf> EncapsulatedInterfaceResponse<'buf> {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::UnknownMeiType` if the MEI type byte is not
    /// recognized.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.is_empty() {
            return Err(DecodeError::Truncated {
                expected: 1,
                actual: 0,
            });
        }
        let mei_type =
            MeiType::from_raw(data[0]).ok_or(DecodeError::UnknownMeiType(data[0]))?;
        Ok(Self {
            mei_type,
            data: &data[1..],
        })
    }
}

impl Encode for EncapsulatedInterfaceResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::EncapsulatedInterfaceTransport.code();
        buf[1] = self.mei_type.code();
        buf[2..2 + self.data.len()].copy_from_slice(self.data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + self.data.len()
    }
}
