//! Response types for register-read function codes (FC 03, 04).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::FunctionCode;

/// Response to a Read Holding Registers request (FC 0x03).
#[derive(Debug)]
pub struct ReadHoldingRegistersResponse<'buf> {
    /// Number of data bytes that follow (should be 2 * register count).
    pub byte_count: u8,
    /// Raw register data in big-endian byte order.
    pub register_data: &'buf [u8],
}

impl<'buf> ReadHoldingRegistersResponse<'buf> {
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

impl Encode for ReadHoldingRegistersResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_byte_count(usize::from(self.byte_count), self.register_data.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadHoldingRegisters.code();
        buf[1] = self.byte_count;
        buf[2..2 + usize::from(self.byte_count)].copy_from_slice(self.register_data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + usize::from(self.byte_count)
    }
}

/// Response to a Read Input Registers request (FC 0x04).
#[derive(Debug)]
pub struct ReadInputRegistersResponse<'buf> {
    /// Number of data bytes that follow (should be 2 * register count).
    pub byte_count: u8,
    /// Raw register data in big-endian byte order.
    pub register_data: &'buf [u8],
}

impl<'buf> ReadInputRegistersResponse<'buf> {
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

impl Encode for ReadInputRegistersResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_byte_count(usize::from(self.byte_count), self.register_data.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadInputRegisters.code();
        buf[1] = self.byte_count;
        buf[2..2 + usize::from(self.byte_count)].copy_from_slice(self.register_data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + usize::from(self.byte_count)
    }
}
