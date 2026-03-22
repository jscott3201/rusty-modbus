//! FIFO queue request: FC 18 (Read FIFO Queue).

use rusty_modbus_types::{Address, FunctionCode};

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

/// FC 0x18 — Read FIFO Queue request.
///
/// Reads the contents of the FIFO queue at the given pointer address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadFifoQueueRequest {
    /// FIFO pointer address.
    pub fifo_pointer_address: Address,
}

impl ReadFifoQueueRequest {
    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 2 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 2 {
            return Err(DecodeError::Truncated {
                expected: 2,
                actual: data.len(),
            });
        }
        let fifo_pointer_address = Address(u16::from_be_bytes([data[0], data[1]]));
        Ok(Self {
            fifo_pointer_address,
        })
    }
}

impl Encode for ReadFifoQueueRequest {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        buf[0] = FunctionCode::ReadFifoQueue.code();
        buf[1..3].copy_from_slice(&self.fifo_pointer_address.0.to_be_bytes());
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        3
    }
}
