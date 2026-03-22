//! Bit-access write requests: FC 05 (Write Single Coil) and FC 0F (Write Multiple Coils).

use rusty_modbus_types::{Address, CoilValue, FunctionCode, Quantity};

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

/// FC 0x05 — Write Single Coil request.
///
/// Sets a single coil at `address` to `value` (ON = 0xFF00, OFF = 0x0000).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteSingleCoilRequest {
    /// Coil address (0-indexed).
    pub address: Address,
    /// Coil value (ON or OFF).
    pub value: CoilValue,
}

impl WriteSingleCoilRequest {
    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 4 bytes.
    /// Returns [`DecodeError::InvalidCoilValue`] if the value is not 0xFF00 or 0x0000.
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

impl Encode for WriteSingleCoilRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::WriteSingleCoil.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.value.to_wire().to_be_bytes());
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        5
    }
}

/// FC 0x0F — Write Multiple Coils request.
///
/// Writes 1..=1968 contiguous coils starting at `address`. Coil values are
/// packed as bits in `coil_values`, LSB of the first byte is the lowest address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteMultipleCoilsRequest<'buf> {
    /// Starting address (0-indexed).
    pub address: Address,
    /// Number of coils to write (1..=1968).
    pub quantity: Quantity,
    /// Number of data bytes that follow.
    pub byte_count: u8,
    /// Packed coil values (bit-packed, LSB first).
    pub coil_values: &'buf [u8],
}

impl<'buf> WriteMultipleCoilsRequest<'buf> {
    /// Maximum quantity for Write Multiple Coils.
    const MAX_QUANTITY: u16 = 1968;

    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is too short.
    /// Returns [`DecodeError::QuantityOutOfRange`] if the quantity is not in 1..=1968.
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
        // Semantic check per spec Figure 21: byte_count must equal ceil(quantity/8).
        let expected_bytes = quantity.div_ceil(8);
        if u16::from(byte_count) != expected_bytes {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: expected_bytes as usize,
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
        let coil_values = &data[5..];
        Ok(Self {
            address,
            quantity: Quantity(quantity),
            byte_count,
            coil_values,
        })
    }
}

impl Encode for WriteMultipleCoilsRequest<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::WriteMultipleCoils.code();
        buf[1..3].copy_from_slice(&self.address.0.to_be_bytes());
        buf[3..5].copy_from_slice(&self.quantity.0.to_be_bytes());
        buf[5] = self.byte_count;
        buf[6..6 + self.coil_values.len()].copy_from_slice(self.coil_values);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC(1) + address(2) + quantity(2) + byte_count(1) + coil_values
        6 + self.coil_values.len()
    }
}
