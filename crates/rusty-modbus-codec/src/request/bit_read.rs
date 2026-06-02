//! Bit-access read requests: FC 01 (Read Coils) and FC 02 (Read Discrete Inputs).

use rusty_modbus_types::{Address, FunctionCode, Quantity};

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

/// FC 0x01 — Read Coils request.
///
/// Reads 1..=2000 contiguous coils starting at `address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadCoilsRequest {
    /// Starting address (0-indexed).
    pub address: Address,
    /// Number of coils to read (1..=2000).
    pub quantity: Quantity,
}

impl ReadCoilsRequest {
    /// Maximum quantity for Read Coils.
    const MAX_QUANTITY: u16 = 2000;

    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 4 bytes.
    /// Returns [`DecodeError::QuantityOutOfRange`] if the quantity is not in 1..=2000.
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

impl Encode for ReadCoilsRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_quantity(self.quantity.0, Self::MAX_QUANTITY)?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadCoils.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.quantity.0.to_be_bytes());
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        5
    }
}

/// FC 0x02 — Read Discrete Inputs request.
///
/// Reads 1..=2000 contiguous discrete inputs starting at `address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadDiscreteInputsRequest {
    /// Starting address (0-indexed).
    pub address: Address,
    /// Number of discrete inputs to read (1..=2000).
    pub quantity: Quantity,
}

impl ReadDiscreteInputsRequest {
    /// Maximum quantity for Read Discrete Inputs.
    const MAX_QUANTITY: u16 = 2000;

    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 4 bytes.
    /// Returns [`DecodeError::QuantityOutOfRange`] if the quantity is not in 1..=2000.
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

impl Encode for ReadDiscreteInputsRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_quantity(self.quantity.0, Self::MAX_QUANTITY)?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadDiscreteInputs.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.quantity.0.to_be_bytes());
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        5
    }
}
