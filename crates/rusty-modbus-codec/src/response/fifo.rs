//! Response type for Read FIFO Queue (FC 18).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::FunctionCode;

/// Response to a Read FIFO Queue request (FC 0x18).
#[derive(Debug)]
pub struct ReadFifoQueueResponse<'buf> {
    /// Total number of bytes following this field.
    pub byte_count: u16,
    /// Number of FIFO register values (0..=31).
    pub fifo_count: u16,
    /// Raw FIFO register data in big-endian byte order.
    pub fifo_values: &'buf [u8],
}

impl<'buf> ReadFifoQueueResponse<'buf> {
    /// Decode from the data bytes following the function code.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError::Truncated` if `data` is too short.
    /// Returns `DecodeError::ByteCountMismatch` if the declared byte count
    /// does not match the remaining data length.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.len() < 4 {
            return Err(DecodeError::Truncated {
                expected: 4,
                actual: data.len(),
            });
        }
        let byte_count = u16::from_be_bytes([data[0], data[1]]);
        let fifo_count = u16::from_be_bytes([data[2], data[3]]);
        let fifo_values = &data[4..];
        // byte_count includes the 2 bytes for fifo_count + the fifo data
        let expected_remaining = usize::from(byte_count).saturating_sub(2);
        if fifo_values.len() != expected_remaining {
            return Err(DecodeError::ByteCountMismatch {
                declared: expected_remaining,
                actual: fifo_values.len(),
            });
        }
        Ok(Self {
            byte_count,
            fifo_count,
            fifo_values,
        })
    }
}

impl Encode for ReadFifoQueueResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::ReadFifoQueue.code();
        let bc = self.byte_count.to_be_bytes();
        buf[1] = bc[0];
        buf[2] = bc[1];
        let fc = self.fifo_count.to_be_bytes();
        buf[3] = fc[0];
        buf[4] = fc[1];
        buf[5..5 + self.fifo_values.len()].copy_from_slice(self.fifo_values);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC + byte_count(2) + fifo_count(2) + fifo_values
        1 + 2 + 2 + self.fifo_values.len()
    }
}
