//! Response types for file record function codes (FC 14, 15).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::FunctionCode;

const FILE_RECORD_MIN_BYTE_COUNT: usize = 0x07;
const FILE_RECORD_MAX_BYTE_COUNT: usize = 0xF5;

fn check_file_record_byte_count(byte_count: u8) -> Result<(), DecodeError> {
    let count = usize::from(byte_count);
    if (FILE_RECORD_MIN_BYTE_COUNT..=FILE_RECORD_MAX_BYTE_COUNT).contains(&count) {
        Ok(())
    } else {
        Err(DecodeError::ByteCountOutOfRange {
            count,
            minimum: FILE_RECORD_MIN_BYTE_COUNT,
            maximum: FILE_RECORD_MAX_BYTE_COUNT,
        })
    }
}

/// Response to a Read File Record request (FC 0x14).
#[derive(Debug)]
pub struct ReadFileRecordResponse<'buf> {
    /// Total number of data bytes that follow.
    pub byte_count: u8,
    /// Raw sub-request response data.
    pub data: &'buf [u8],
}

impl<'buf> ReadFileRecordResponse<'buf> {
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
        check_file_record_byte_count(byte_count)?;
        let payload = &data[1..];
        if payload.len() != usize::from(byte_count) {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: payload.len(),
            });
        }
        Ok(Self {
            byte_count,
            data: payload,
        })
    }
}

impl Encode for ReadFileRecordResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_byte_count_range(
            usize::from(self.byte_count),
            FILE_RECORD_MIN_BYTE_COUNT,
            FILE_RECORD_MAX_BYTE_COUNT,
        )?;
        EncodeError::check_byte_count(usize::from(self.byte_count), self.data.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadFileRecord.code();
        buf[1] = self.byte_count;
        buf[2..2 + usize::from(self.byte_count)].copy_from_slice(self.data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + usize::from(self.byte_count)
    }
}

/// Response to a Write File Record request (FC 0x15).
#[derive(Debug)]
pub struct WriteFileRecordResponse<'buf> {
    /// Total number of data bytes that follow.
    pub byte_count: u8,
    /// Raw sub-request response data.
    pub data: &'buf [u8],
}

impl<'buf> WriteFileRecordResponse<'buf> {
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
        check_file_record_byte_count(byte_count)?;
        let payload = &data[1..];
        if payload.len() != usize::from(byte_count) {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: payload.len(),
            });
        }
        Ok(Self {
            byte_count,
            data: payload,
        })
    }
}

impl Encode for WriteFileRecordResponse<'_> {
    fn encode_into(&self, buf: &mut [u8]) -> Result<usize, EncodeError> {
        let len = self.encoded_len();
        if buf.len() < len {
            return Err(EncodeError::BufferTooSmall {
                required: len,
                available: buf.len(),
            });
        }
        EncodeError::check_byte_count_range(
            usize::from(self.byte_count),
            FILE_RECORD_MIN_BYTE_COUNT,
            FILE_RECORD_MAX_BYTE_COUNT,
        )?;
        EncodeError::check_byte_count(usize::from(self.byte_count), self.data.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::WriteFileRecord.code();
        buf[1] = self.byte_count;
        buf[2..2 + usize::from(self.byte_count)].copy_from_slice(self.data);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        1 + 1 + usize::from(self.byte_count)
    }
}
