//! Response types for bit-write function codes (FC 05, 0F).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::{Address, CoilValue, FunctionCode, Quantity};

/// Response to a Write Single Coil request (FC 0x05).
///
/// This is an echo of the request.
#[derive(Debug)]
pub struct WriteSingleCoilResponse {
    /// Address of the coil that was written.
    pub address: Address,
    /// Value that was written to the coil.
    pub value: CoilValue,
}

impl WriteSingleCoilResponse {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::InvalidCoilValue` if the coil value is not
    /// 0xFF00 or 0x0000.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 4 {
            return Err(DecodeError::Truncated {
                expected: 4,
                actual: data.len(),
            });
        }
        let address = Address(u16::from_be_bytes([data[0], data[1]]));
        let raw_value = u16::from_be_bytes([data[2], data[3]]);
        let value =
            CoilValue::from_wire(raw_value).ok_or(DecodeError::InvalidCoilValue(raw_value))?;
        Ok(Self { address, value })
    }
}

impl Encode for WriteSingleCoilResponse {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::WriteSingleCoil.code();
        let addr = self.address.0.to_be_bytes();
        buf[1] = addr[0];
        buf[2] = addr[1];
        let val = self.value.to_wire().to_be_bytes();
        buf[3] = val[0];
        buf[4] = val[1];
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 4
    }
}

/// Response to a Write Multiple Coils request (FC 0x0F).
#[derive(Debug)]
pub struct WriteMultipleCoilsResponse {
    /// Starting address of coils that were written.
    pub address: Address,
    /// Number of coils that were written.
    pub quantity: Quantity,
}

impl WriteMultipleCoilsResponse {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 4 {
            return Err(DecodeError::Truncated {
                expected: 4,
                actual: data.len(),
            });
        }
        let address = Address(u16::from_be_bytes([data[0], data[1]]));
        let quantity = Quantity(u16::from_be_bytes([data[2], data[3]]));
        Ok(Self { address, quantity })
    }
}

impl Encode for WriteMultipleCoilsResponse {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::WriteMultipleCoils.code();
        let addr = self.address.0.to_be_bytes();
        buf[1] = addr[0];
        buf[2] = addr[1];
        let qty = self.quantity.0.to_be_bytes();
        buf[3] = qty[0];
        buf[4] = qty[1];
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 4
    }
}
