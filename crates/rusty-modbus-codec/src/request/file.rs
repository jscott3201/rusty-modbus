//! File record requests: FC 14 (Read File Record) and FC 15 (Write File Record).

use rusty_modbus_types::FunctionCode;

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

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

/// A single file sub-request record (7 bytes on the wire).
///
/// Used inside both Read File Record and Write File Record requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileSubRequest {
    /// Reference type — must be 6.
    pub reference_type: u8,
    /// File number.
    pub file_number: u16,
    /// Starting record number within the file.
    pub record_number: u16,
    /// Number of records to read/write.
    pub record_length: u16,
}

impl FileSubRequest {
    /// Decode a single sub-request from a 7-byte slice.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is shorter than 7 bytes.
    /// Returns [`DecodeError::InvalidReferenceType`] if the reference type is not 6.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < 7 {
            return Err(DecodeError::Truncated {
                expected: 7,
                actual: data.len(),
            });
        }
        let reference_type = data[0];
        if reference_type != 6 {
            return Err(DecodeError::InvalidReferenceType(reference_type));
        }
        let file_number = u16::from_be_bytes([data[1], data[2]]);
        let record_number = u16::from_be_bytes([data[3], data[4]]);
        let record_length = u16::from_be_bytes([data[5], data[6]]);
        Ok(Self {
            reference_type,
            file_number,
            record_number,
            record_length,
        })
    }
}

/// FC 0x14 — Read File Record request.
///
/// Contains one or more 7-byte sub-request records packed in `sub_requests`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadFileRecordRequest<'buf> {
    /// Total byte count of the sub-request data that follows.
    pub byte_count: u8,
    /// Raw sub-request data (each sub-request is 7 bytes).
    pub sub_requests: &'buf [u8],
}

impl<'buf> ReadFileRecordRequest<'buf> {
    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is too short.
    /// Returns [`DecodeError::ByteCountMismatch`] if the declared byte count does not
    /// match the remaining data length.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.is_empty() {
            return Err(DecodeError::Truncated {
                expected: 1,
                actual: 0,
            });
        }
        let byte_count = data[0];
        check_file_record_byte_count(byte_count)?;
        let remaining = data.len() - 1;
        if byte_count as usize != remaining {
            return Err(DecodeError::ByteCountMismatch {
                declared: byte_count as usize,
                actual: remaining,
            });
        }
        let sub_requests = &data[1..];
        Ok(Self {
            byte_count,
            sub_requests,
        })
    }
}

impl Encode for ReadFileRecordRequest<'_> {
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
        EncodeError::check_byte_count(usize::from(self.byte_count), self.sub_requests.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::ReadFileRecord.code();
        buf[1] = self.byte_count;
        buf[2..2 + self.sub_requests.len()].copy_from_slice(self.sub_requests);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC(1) + byte_count(1) + sub_requests
        2 + self.sub_requests.len()
    }
}

/// FC 0x15 — Write File Record request.
///
/// Contains one or more sub-request records packed in `sub_requests`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteFileRecordRequest<'buf> {
    /// Total byte count of the sub-request data that follows.
    pub byte_count: u8,
    /// Raw sub-request data.
    pub sub_requests: &'buf [u8],
}

impl<'buf> WriteFileRecordRequest<'buf> {
    /// Decode from PDU data after the function code byte.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Truncated`] if `data` is too short.
    /// Returns [`DecodeError::ByteCountMismatch`] if the declared byte count does not
    /// match the remaining data length.
    pub fn decode(data: &'buf [u8]) -> Result<Self, DecodeError> {
        if data.is_empty() {
            return Err(DecodeError::Truncated {
                expected: 1,
                actual: 0,
            });
        }
        let byte_count = data[0];
        check_file_record_byte_count(byte_count)?;
        let remaining = data.len() - 1;
        if byte_count as usize != remaining {
            return Err(DecodeError::ByteCountMismatch {
                declared: byte_count as usize,
                actual: remaining,
            });
        }
        let sub_requests = &data[1..];
        Ok(Self {
            byte_count,
            sub_requests,
        })
    }
}

impl Encode for WriteFileRecordRequest<'_> {
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
        EncodeError::check_byte_count(usize::from(self.byte_count), self.sub_requests.len())?;
        EncodeError::check_pdu_len(len)?;
        buf[0] = FunctionCode::WriteFileRecord.code();
        buf[1] = self.byte_count;
        buf[2..2 + self.sub_requests.len()].copy_from_slice(self.sub_requests);
        Ok(len)
    }

    fn encoded_len(&self) -> usize {
        // FC(1) + byte_count(1) + sub_requests
        2 + self.sub_requests.len()
    }
}
