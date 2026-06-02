//! Response types for bit-read function codes (FC 01, 02).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::FunctionCode;

/// Response to a Read Coils request (FC 0x01).
///
/// Contains bit-packed coil status data in LSB-first order.
#[derive(Debug)]
pub struct ReadCoilsResponse<'buf> {
    /// Number of data bytes that follow.
    pub byte_count: u8,
    /// Bit-packed coil status, LSB-first within each byte.
    pub coil_status: &'buf [u8],
}

impl<'buf> ReadCoilsResponse<'buf> {
    /// Returns the state of the coil at the given zero-based index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range for the coil status data.
    #[must_use]
    pub fn coil(&self, index: usize) -> bool {
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        (self.coil_status[byte_idx] >> bit_idx) & 1 == 1
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
        let coil_status = &data[1..];
        if coil_status.len() != usize::from(byte_count) {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: coil_status.len(),
            });
        }
        Ok(Self {
            byte_count,
            coil_status,
        })
    }
}

impl Encode for ReadCoilsResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_byte_count(usize::from(self.byte_count), self.coil_status.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadCoils.code();
        buf[1] = self.byte_count;
        buf[2..2 + usize::from(self.byte_count)].copy_from_slice(self.coil_status);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + usize::from(self.byte_count)
    }
}

/// Response to a Read Discrete Inputs request (FC 0x02).
///
/// Contains bit-packed discrete input status data in LSB-first order.
#[derive(Debug)]
pub struct ReadDiscreteInputsResponse<'buf> {
    /// Number of data bytes that follow.
    pub byte_count: u8,
    /// Bit-packed input status, LSB-first within each byte.
    pub coil_status: &'buf [u8],
}

impl<'buf> ReadDiscreteInputsResponse<'buf> {
    /// Returns the state of the discrete input at the given zero-based index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range for the input status data.
    #[must_use]
    pub fn coil(&self, index: usize) -> bool {
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        (self.coil_status[byte_idx] >> bit_idx) & 1 == 1
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
        let coil_status = &data[1..];
        if coil_status.len() != usize::from(byte_count) {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: coil_status.len(),
            });
        }
        Ok(Self {
            byte_count,
            coil_status,
        })
    }
}

impl Encode for ReadDiscreteInputsResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_byte_count(usize::from(self.byte_count), self.coil_status.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadDiscreteInputs.code();
        buf[1] = self.byte_count;
        buf[2..2 + usize::from(self.byte_count)].copy_from_slice(self.coil_status);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + usize::from(self.byte_count)
    }
}
