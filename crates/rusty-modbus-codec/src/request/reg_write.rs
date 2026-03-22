//! Register-access write requests: FC 06, 10, 16, 17.

use rusty_modbus_types::{Address, FunctionCode, Quantity};

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

/// FC 0x06 — Write Single Register request.
///
/// Writes a single 16-bit value to the holding register at `address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteSingleRegisterRequest {
    /// Register address (0-indexed).
    pub address: Address,
    /// Value to write.
    pub value: u16,
}

impl WriteSingleRegisterRequest {
    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 4 bytes.
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

impl Encode for WriteSingleRegisterRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::WriteSingleRegister.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.value.to_be_bytes());
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        5
    }
}

/// FC 0x10 — Write Multiple Registers request.
///
/// Writes 1..=123 contiguous holding registers starting at `address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteMultipleRegistersRequest<'buf> {
    /// Starting address (0-indexed).
    pub address: Address,
    /// Number of registers to write (1..=123).
    pub quantity: Quantity,
    /// Number of data bytes that follow (should be `quantity * 2`).
    pub byte_count: u8,
    /// Register values in big-endian byte order.
    pub register_values: &'buf [u8],
}

impl<'buf> WriteMultipleRegistersRequest<'buf> {
    /// Maximum quantity for Write Multiple Registers.
    const MAX_QUANTITY: u16 = 123;

    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is too short.
    /// Returns [`DecodeError::QuantityOutOfRange`] if the quantity is not in 1..=123.
    /// Returns [`DecodeError::ByteCountMismatch`] if the declared byte count does not
    /// match the remaining data length.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.len() < 5 {
            return Err(DecodeError::Truncated {
                expected: 5,
                actual: data.len(),
            });
        }
        let address = Address(u16::from_be_bytes([data[0], data[1]]));
        let quantity = u16::from_be_bytes([data[2], data[3]]);
        if quantity == 0 || quantity > Self::MAX_QUANTITY {
            return Err(DecodeError::QuantityOutOfRange { quantity });
        }
        let byte_count = data[4];
        // Semantic check per spec Figure 22: byte_count must equal quantity × 2.
        if u16::from(byte_count) != quantity * 2 {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: (quantity * 2) as usize,
            });
        }
        // Wire check: declared byte_count must match actual remaining data.
        let remaining = data.len() - 5;
        if byte_count as usize != remaining {
            return Err(DecodeError::ByteCountMismatch {
                declared: byte_count as usize,
                actual: remaining,
            });
        }
        let register_values = &data[5..];
        Ok(Self {
            address,
            quantity: Quantity(quantity),
            byte_count,
            register_values,
        })
    }
}

impl Encode for WriteMultipleRegistersRequest<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::WriteMultipleRegisters.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.quantity.0.to_be_bytes());
        buf[5] = self.byte_count;
        buf[6..6 + self.register_values.len()].copy_from_slice(self.register_values);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC(1) + address(2) + quantity(2) + byte_count(1) + register_values
        6 + self.register_values.len()
    }
}

/// FC 0x16 — Mask Write Register request.
///
/// Modifies a single holding register using AND and OR masks:
/// `result = (current AND and_mask) OR (or_mask AND (NOT and_mask))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskWriteRegisterRequest {
    /// Register address (0-indexed).
    pub address: Address,
    /// AND mask.
    pub and_mask: u16,
    /// OR mask.
    pub or_mask: u16,
}

impl MaskWriteRegisterRequest {
    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 6 bytes.
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

impl Encode for MaskWriteRegisterRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::MaskWriteRegister.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.and_mask.to_be_bytes());
        buf[5..7].copy_from_slice(&self.or_mask.to_be_bytes());
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        7
    }
}

/// FC 0x17 — Read/Write Multiple Registers request.
///
/// Atomically reads `read_quantity` registers starting at `read_address` and writes
/// `write_quantity` registers starting at `write_address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadWriteMultipleRegistersRequest<'buf> {
    /// Starting address for reading (0-indexed).
    pub read_address: Address,
    /// Number of registers to read (1..=125).
    pub read_quantity: Quantity,
    /// Starting address for writing (0-indexed).
    pub write_address: Address,
    /// Number of registers to write (1..=121).
    pub write_quantity: Quantity,
    /// Number of bytes in the write data (should be `write_quantity * 2`).
    pub write_byte_count: u8,
    /// Register values to write in big-endian byte order.
    pub write_register_values: &'buf [u8],
}

impl<'buf> ReadWriteMultipleRegistersRequest<'buf> {
    /// Maximum read quantity.
    const MAX_READ_QUANTITY: u16 = 125;
    /// Maximum write quantity.
    const MAX_WRITE_QUANTITY: u16 = 121;

    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is too short.
    /// Returns [`DecodeError::QuantityOutOfRange`] if either quantity is out of range.
    /// Returns [`DecodeError::ByteCountMismatch`] if the declared byte count does not
    /// match the remaining data length.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.len() < 9 {
            return Err(DecodeError::Truncated {
                expected: 9,
                actual: data.len(),
            });
        }
        let read_address = Address(u16::from_be_bytes([data[0], data[1]]));
        let read_quantity = u16::from_be_bytes([data[2], data[3]]);
        if read_quantity == 0 || read_quantity > Self::MAX_READ_QUANTITY {
            return Err(DecodeError::QuantityOutOfRange {
                quantity: read_quantity,
            });
        }
        let write_address = Address(u16::from_be_bytes([data[4], data[5]]));
        let write_quantity = u16::from_be_bytes([data[6], data[7]]);
        if write_quantity == 0 || write_quantity > Self::MAX_WRITE_QUANTITY {
            return Err(DecodeError::QuantityOutOfRange {
                quantity: write_quantity,
            });
        }
        let write_byte_count = data[8];
        // Semantic check per spec Figure 27: byte_count must equal write_quantity × 2.
        if u16::from(write_byte_count) != write_quantity * 2 {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(write_byte_count),
                actual: (write_quantity * 2) as usize,
            });
        }
        // Wire check: declared byte_count must match actual remaining data.
        let remaining = data.len() - 9;
        if write_byte_count as usize != remaining {
            return Err(DecodeError::ByteCountMismatch {
                declared: write_byte_count as usize,
                actual: remaining,
            });
        }
        let write_register_values = &data[9..];
        Ok(Self {
            read_address,
            read_quantity: Quantity(read_quantity),
            write_address,
            write_quantity: Quantity(write_quantity),
            write_byte_count,
            write_register_values,
        })
    }
}

impl Encode for ReadWriteMultipleRegistersRequest<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::ReadWriteMultipleRegisters.code();
        buf[1..3].copy_from_slice(&self.read_address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.read_quantity.0.to_be_bytes());
        buf[5..7].copy_from_slice(&self.write_address.0.to_be_bytes());
        buf[7..9].copy_from_slice(&self.write_quantity.0.to_be_bytes());
        buf[9] = self.write_byte_count;
        buf[10..10 + self.write_register_values.len()]
            .copy_from_slice(self.write_register_values);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC(1) + read_addr(2) + read_qty(2) + write_addr(2) + write_qty(2) + byte_count(1) + data
        10 + self.write_register_values.len()
    }
}
