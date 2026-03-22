//! Register-access read requests: FC 03 (Read Holding Registers) and FC 04 (Read Input Registers).

use modbus_types::{Address, FunctionCode, Quantity};

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

/// FC 0x03 — Read Holding Registers request.
///
/// Reads 1..=125 contiguous holding registers starting at `address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadHoldingRegistersRequest {
    /// Starting address (0-indexed).
    pub address: Address,
    /// Number of registers to read (1..=125).
    pub quantity: Quantity,
}

impl ReadHoldingRegistersRequest {
    /// Maximum quantity for Read Holding Registers.
    const MAX_QUANTITY: u16 = 125;

    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 4 bytes.
    /// Returns [`DecodeError::QuantityOutOfRange`] if the quantity is not in 1..=125.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 4 {
            return Err(DecodeError::Truncated {
                expected: 4,
                actual: data.len(),
            });
        }
        let address = Address(u16::from_be_bytes([data[0], data[1]]));
        let quantity = u16::from_be_bytes([data[2], data[3]]);
        if quantity == 0 || quantity > Self::MAX_QUANTITY {
            return Err(DecodeError::QuantityOutOfRange { quantity });
        }
        Ok(Self {
            address,
            quantity: Quantity(quantity),
        })
    }
}

impl Encode for ReadHoldingRegistersRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::ReadHoldingRegisters.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.quantity.0.to_be_bytes());
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        5
    }
}

/// FC 0x04 — Read Input Registers request.
///
/// Reads 1..=125 contiguous input registers starting at `address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadInputRegistersRequest {
    /// Starting address (0-indexed).
    pub address: Address,
    /// Number of registers to read (1..=125).
    pub quantity: Quantity,
}

impl ReadInputRegistersRequest {
    /// Maximum quantity for Read Input Registers.
    const MAX_QUANTITY: u16 = 125;

    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 4 bytes.
    /// Returns [`DecodeError::QuantityOutOfRange`] if the quantity is not in 1..=125.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 4 {
            return Err(DecodeError::Truncated {
                expected: 4,
                actual: data.len(),
            });
        }
        let address = Address(u16::from_be_bytes([data[0], data[1]]));
        let quantity = u16::from_be_bytes([data[2], data[3]]);
        if quantity == 0 || quantity > Self::MAX_QUANTITY {
            return Err(DecodeError::QuantityOutOfRange { quantity });
        }
        Ok(Self {
            address,
            quantity: Quantity(quantity),
        })
    }
}

impl Encode for ReadInputRegistersRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::ReadInputRegisters.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.quantity.0.to_be_bytes());
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        5
    }
}
