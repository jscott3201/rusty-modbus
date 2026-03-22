//! Response types for register-write function codes (FC 06, 10, 16, 17).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use modbus_types::{Address, FunctionCode, Quantity};

/// Response to a Write Single Register request (FC 0x06).
///
/// This is an echo of the request.
#[derive(Debug)]
pub struct WriteSingleRegisterResponse {
    /// Address of the register that was written.
    pub address: Address,
    /// Value that was written to the register.
    pub value: u16,
}

impl WriteSingleRegisterResponse {
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
        let value = u16::from_be_bytes([data[2], data[3]]);
        Ok(Self { address, value })
    }
}

impl Encode for WriteSingleRegisterResponse {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::WriteSingleRegister.code();
        let addr = self.address.0.to_be_bytes();
        buf[1] = addr[0];
        buf[2] = addr[1];
        let val = self.value.to_be_bytes();
        buf[3] = val[0];
        buf[4] = val[1];
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 4
    }
}

/// Response to a Write Multiple Registers request (FC 0x10).
#[derive(Debug)]
pub struct WriteMultipleRegistersResponse {
    /// Starting address of registers that were written.
    pub address: Address,
    /// Number of registers that were written.
    pub quantity: Quantity,
}

impl WriteMultipleRegistersResponse {
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

impl Encode for WriteMultipleRegistersResponse {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::WriteMultipleRegisters.code();
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

/// Response to a Mask Write Register request (FC 0x16).
///
/// This is an echo of the request.
#[derive(Debug)]
pub struct MaskWriteRegisterResponse {
    /// Address of the register.
    pub address: Address,
    /// AND mask applied to the register.
    pub and_mask: u16,
    /// OR mask applied to the register.
    pub or_mask: u16,
}

impl MaskWriteRegisterResponse {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 6 {
            return Err(DecodeError::Truncated {
                expected: 6,
                actual: data.len(),
            });
        }
        let address = Address(u16::from_be_bytes([data[0], data[1]]));
        let and_mask = u16::from_be_bytes([data[2], data[3]]);
        let or_mask = u16::from_be_bytes([data[4], data[5]]);
        Ok(Self {
            address,
            and_mask,
            or_mask,
        })
    }
}

impl Encode for MaskWriteRegisterResponse {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::MaskWriteRegister.code();
        let addr = self.address.0.to_be_bytes();
        buf[1] = addr[0];
        buf[2] = addr[1];
        let and_m = self.and_mask.to_be_bytes();
        buf[3] = and_m[0];
        buf[4] = and_m[1];
        let or_m = self.or_mask.to_be_bytes();
        buf[5] = or_m[0];
        buf[6] = or_m[1];
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 6
    }
}

/// Response to a Read/Write Multiple Registers request (FC 0x17).
#[derive(Debug)]
pub struct ReadWriteMultipleRegistersResponse<'buf> {
    /// Number of data bytes that follow (should be 2 * register count).
    pub byte_count: u8,
    /// Raw register data in big-endian byte order.
    pub register_data: &'buf [u8],
}

impl<'buf> ReadWriteMultipleRegistersResponse<'buf> {
    /// Returns the number of registers in this response.
    #[must_use]
    pub fn count(&self) -> usize {
        self.register_data.len() / 2
    }

    /// Returns the register value at the given zero-based index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    #[must_use]
    pub fn register(&self, index: usize) -> u16 {
        let off = index * 2;
        u16::from_be_bytes([self.register_data[off], self.register_data[off + 1]])
    }

    /// Returns an iterator over all register values.
    pub fn registers(&self) -> impl Iterator<Item = u16> + '_ {
        self.register_data
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
    }

    /// Returns the raw register data bytes.
    #[must_use]
    pub fn raw(&self) -> &'buf [u8] {
        self.register_data
    }

    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::ByteCountMismatch` if the declared byte count
    /// does not match the remaining data length.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.is_empty() {
            return Err(DecodeError::Truncated {
                expected: 1,
                actual: 0,
            });
        }
        let byte_count = data[0];
        let register_data = &data[1..];
        if register_data.len() != usize::from(byte_count) {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: register_data.len(),
            });
        }
        Ok(Self {
            byte_count,
            register_data,
        })
    }
}

impl Encode for ReadWriteMultipleRegistersResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::ReadWriteMultipleRegisters.code();
        buf[1] = self.byte_count;
        buf[2..2 + usize::from(self.byte_count)].copy_from_slice(self.register_data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + usize::from(self.byte_count)
    }
}
