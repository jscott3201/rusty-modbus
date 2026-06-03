//! Response types for file record function codes (FC 14, 15).

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;
use rusty_modbus_types::FunctionCode;

const READ_FILE_RECORD_RESPONSE_MIN_BYTE_COUNT: usize = 0x04;
const READ_FILE_RECORD_RESPONSE_MAX_BYTE_COUNT: usize = 0xFA;
const WRITE_FILE_RECORD_MIN_BYTE_COUNT: usize = 0x09;
const WRITE_FILE_RECORD_MAX_BYTE_COUNT: usize = 0xFB;
const FILE_RECORD_REFERENCE_TYPE: u8 = 0x06;
const MIN_FILE_NUMBER: u16 = 0x0001;
const MAX_RECORD_NUMBER: u16 = 0x270F;
const RECORD_COUNT: usize = 0x2710;

fn check_file_record_byte_count(
    byte_count: u8,
    minimum: usize,
    maximum: usize,
) -> Result<(), DecodeError> {
    let count = usize::from(byte_count);
    if (minimum..=maximum).contains(&count) {
        Ok(())
    } else {
        Err(DecodeError::ByteCountOutOfRange {
            count,
            minimum,
            maximum,
        })
    }
}

fn check_file_record_range(
    file_number: u16,
    record_number: u16,
    record_length: u16,
) -> Result<(), DecodeError> {
    let end = usize::from(record_number)
        .checked_add(usize::from(record_length))
        .ok_or(DecodeError::FileRecordOutOfRange {
            file_number,
            record_number,
            record_length,
        })?;
    if file_number < MIN_FILE_NUMBER
        || record_length == 0
        || record_number > MAX_RECORD_NUMBER
        || end > RECORD_COUNT
    {
        return Err(DecodeError::FileRecordOutOfRange {
            file_number,
            record_number,
            record_length,
        });
    }
    Ok(())
}

fn check_file_record_range_encode(
    file_number: u16,
    record_number: u16,
    record_length: u16,
) -> Result<(), EncodeError> {
    let end = usize::from(record_number)
        .checked_add(usize::from(record_length))
        .ok_or(EncodeError::FileRecordOutOfRange {
            file_number,
            record_number,
            record_length,
        })?;
    if file_number < MIN_FILE_NUMBER
        || record_length == 0
        || record_number > MAX_RECORD_NUMBER
        || end > RECORD_COUNT
    {
        return Err(EncodeError::FileRecordOutOfRange {
            file_number,
            record_number,
            record_length,
        });
    }
    Ok(())
}

fn validate_read_file_response_payload(payload: &[u8]) -> Result<(), DecodeError> {
    let mut remaining = payload;
    while !remaining.is_empty() {
        let response_len = usize::from(remaining[0]);
        if response_len < 3 || response_len % 2 == 0 {
            return Err(DecodeError::InvalidFileRecordLength {
                length: response_len,
            });
        }
        let group_len = 1 + response_len;
        if remaining.len() < group_len {
            return Err(DecodeError::ByteCountMismatch {
                declared: group_len,
                actual: remaining.len(),
            });
        }
        let reference_type = remaining[1];
        if reference_type != FILE_RECORD_REFERENCE_TYPE {
            return Err(DecodeError::InvalidReferenceType(reference_type));
        }
        remaining = &remaining[group_len..];
    }
    Ok(())
}

fn validate_read_file_response_payload_encode(payload: &[u8]) -> Result<(), EncodeError> {
    let mut remaining = payload;
    while !remaining.is_empty() {
        let response_len = usize::from(remaining[0]);
        if response_len < 3 || response_len % 2 == 0 {
            return Err(EncodeError::InvalidFileRecordLength {
                length: response_len,
            });
        }
        let group_len = 1 + response_len;
        if remaining.len() < group_len {
            return Err(EncodeError::ByteCountMismatch {
                declared: group_len,
                actual: remaining.len(),
            });
        }
        let reference_type = remaining[1];
        if reference_type != FILE_RECORD_REFERENCE_TYPE {
            return Err(EncodeError::InvalidReferenceType(reference_type));
        }
        remaining = &remaining[group_len..];
    }
    Ok(())
}

fn validate_write_file_payload(payload: &[u8]) -> Result<(), DecodeError> {
    let mut remaining = payload;
    while !remaining.is_empty() {
        if remaining.len() < 7 {
            return Err(DecodeError::InvalidFileRecordLength {
                length: remaining.len(),
            });
        }
        let reference_type = remaining[0];
        if reference_type != FILE_RECORD_REFERENCE_TYPE {
            return Err(DecodeError::InvalidReferenceType(reference_type));
        }
        let file_number = u16::from_be_bytes([remaining[1], remaining[2]]);
        let record_number = u16::from_be_bytes([remaining[3], remaining[4]]);
        let record_length = u16::from_be_bytes([remaining[5], remaining[6]]);
        check_file_record_range(file_number, record_number, record_length)?;
        let group_len = 7 + usize::from(record_length) * 2;
        if remaining.len() < group_len {
            return Err(DecodeError::ByteCountMismatch {
                declared: group_len,
                actual: remaining.len(),
            });
        }
        remaining = &remaining[group_len..];
    }
    Ok(())
}

fn validate_write_file_payload_encode(payload: &[u8]) -> Result<(), EncodeError> {
    let mut remaining = payload;
    while !remaining.is_empty() {
        if remaining.len() < 7 {
            return Err(EncodeError::InvalidFileRecordLength {
                length: remaining.len(),
            });
        }
        let reference_type = remaining[0];
        if reference_type != FILE_RECORD_REFERENCE_TYPE {
            return Err(EncodeError::InvalidReferenceType(reference_type));
        }
        let file_number = u16::from_be_bytes([remaining[1], remaining[2]]);
        let record_number = u16::from_be_bytes([remaining[3], remaining[4]]);
        let record_length = u16::from_be_bytes([remaining[5], remaining[6]]);
        check_file_record_range_encode(file_number, record_number, record_length)?;
        let group_len = 7 + usize::from(record_length) * 2;
        if remaining.len() < group_len {
            return Err(EncodeError::ByteCountMismatch {
                declared: group_len,
                actual: remaining.len(),
            });
        }
        remaining = &remaining[group_len..];
    }
    Ok(())
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
        check_file_record_byte_count(
            byte_count,
            READ_FILE_RECORD_RESPONSE_MIN_BYTE_COUNT,
            READ_FILE_RECORD_RESPONSE_MAX_BYTE_COUNT,
        )?;
        let payload = &data[1..];
        if payload.len() != usize::from(byte_count) {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: payload.len(),
            });
        }
        validate_read_file_response_payload(payload)?;
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
            READ_FILE_RECORD_RESPONSE_MIN_BYTE_COUNT,
            READ_FILE_RECORD_RESPONSE_MAX_BYTE_COUNT,
        )?;
        EncodeError::check_byte_count(usize::from(self.byte_count), self.data.len())?;
        validate_read_file_response_payload_encode(self.data)?;
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
        check_file_record_byte_count(
            byte_count,
            WRITE_FILE_RECORD_MIN_BYTE_COUNT,
            WRITE_FILE_RECORD_MAX_BYTE_COUNT,
        )?;
        let payload = &data[1..];
        if payload.len() != usize::from(byte_count) {
            return Err(DecodeError::ByteCountMismatch {
                declared: usize::from(byte_count),
                actual: payload.len(),
            });
        }
        validate_write_file_payload(payload)?;
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
            WRITE_FILE_RECORD_MIN_BYTE_COUNT,
            WRITE_FILE_RECORD_MAX_BYTE_COUNT,
        )?;
        EncodeError::check_byte_count(usize::from(self.byte_count), self.data.len())?;
        validate_write_file_payload_encode(self.data)?;
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
