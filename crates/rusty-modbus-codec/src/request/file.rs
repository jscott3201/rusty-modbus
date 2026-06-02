//! File record requests: FC 14 (Read File Record) and FC 15 (Write File Record).

use rusty_modbus_types::FunctionCode;

use crate::error::{DecodeError, EncodeError};
use crate::request::Encode;

const READ_FILE_RECORD_MIN_BYTE_COUNT: usize = 0x07;
const READ_FILE_RECORD_MAX_BYTE_COUNT: usize = 0xF5;
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

fn validate_read_file_sub_requests(sub_requests: &[u8]) -> Result<(), DecodeError> {
    if !sub_requests.len().is_multiple_of(7) {
        return Err(DecodeError::InvalidFileRecordLength {
            length: sub_requests.len(),
        });
    }
    for chunk in sub_requests.chunks_exact(7) {
        FileSubRequest::decode(chunk)?;
    }
    Ok(())
}

fn validate_read_file_sub_requests_encode(sub_requests: &[u8]) -> Result<(), EncodeError> {
    if !sub_requests.len().is_multiple_of(7) {
        return Err(EncodeError::InvalidFileRecordLength {
            length: sub_requests.len(),
        });
    }
    for chunk in sub_requests.chunks_exact(7) {
        validate_file_sub_request_encode(chunk)?;
    }
    Ok(())
}

fn validate_write_file_sub_requests(sub_requests: &[u8]) -> Result<(), DecodeError> {
    let mut remaining = sub_requests;
    while !remaining.is_empty() {
        if remaining.len() < 7 {
            return Err(DecodeError::InvalidFileRecordLength {
                length: remaining.len(),
            });
        }
        let sub = FileSubRequest::decode(&remaining[..7])?;
        let value_bytes = usize::from(sub.record_length) * 2;
        let group_len = 7 + value_bytes;
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

fn validate_write_file_sub_requests_encode(sub_requests: &[u8]) -> Result<(), EncodeError> {
    let mut remaining = sub_requests;
    while !remaining.is_empty() {
        if remaining.len() < 7 {
            return Err(EncodeError::InvalidFileRecordLength {
                length: remaining.len(),
            });
        }
        let sub = validate_file_sub_request_encode(&remaining[..7])?;
        let value_bytes = usize::from(sub.record_length) * 2;
        let group_len = 7 + value_bytes;
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

fn validate_file_sub_request_encode(data: &[u8]) -> Result<FileSubRequest, EncodeError> {
    if data.len() < 7 {
        return Err(EncodeError::InvalidFileRecordLength { length: data.len() });
    }
    let reference_type = data[0];
    if reference_type != FILE_RECORD_REFERENCE_TYPE {
        return Err(EncodeError::InvalidReferenceType(reference_type));
    }
    let file_number = u16::from_be_bytes([data[1], data[2]]);
    let record_number = u16::from_be_bytes([data[3], data[4]]);
    let record_length = u16::from_be_bytes([data[5], data[6]]);
    check_file_record_range_encode(file_number, record_number, record_length)?;
    Ok(FileSubRequest {
        reference_type,
        file_number,
        record_number,
        record_length,
    })
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
    /// Returns [`DecodeError::LengthMismatch`] if `data` has extra bytes.
    /// Returns [`DecodeError::InvalidReferenceType`] if the reference type is not 6.
    /// Returns [`DecodeError::FileRecordOutOfRange`] if file or record fields are
    /// outside the Modbus file-record model.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        DecodeError::check_exact_len(data, 7)?;
        let reference_type = data[0];
        if reference_type != FILE_RECORD_REFERENCE_TYPE {
            return Err(DecodeError::InvalidReferenceType(reference_type));
        }
        let file_number = u16::from_be_bytes([data[1], data[2]]);
        let record_number = u16::from_be_bytes([data[3], data[4]]);
        let record_length = u16::from_be_bytes([data[5], data[6]]);
        check_file_record_range(file_number, record_number, record_length)?;
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
        check_file_record_byte_count(
            byte_count,
            READ_FILE_RECORD_MIN_BYTE_COUNT,
            READ_FILE_RECORD_MAX_BYTE_COUNT,
        )?;
        let remaining = data.len() - 1;
        if byte_count as usize != remaining {
            return Err(DecodeError::ByteCountMismatch {
                declared: byte_count as usize,
                actual: remaining,
            });
        }
        let sub_requests = &data[1..];
        validate_read_file_sub_requests(sub_requests)?;
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
            READ_FILE_RECORD_MIN_BYTE_COUNT,
            READ_FILE_RECORD_MAX_BYTE_COUNT,
        )?;
        EncodeError::check_byte_count(usize::from(self.byte_count), self.sub_requests.len())?;
        validate_read_file_sub_requests_encode(self.sub_requests)?;
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
        check_file_record_byte_count(
            byte_count,
            WRITE_FILE_RECORD_MIN_BYTE_COUNT,
            WRITE_FILE_RECORD_MAX_BYTE_COUNT,
        )?;
        let remaining = data.len() - 1;
        if byte_count as usize != remaining {
            return Err(DecodeError::ByteCountMismatch {
                declared: byte_count as usize,
                actual: remaining,
            });
        }
        let sub_requests = &data[1..];
        validate_write_file_sub_requests(sub_requests)?;
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
            WRITE_FILE_RECORD_MIN_BYTE_COUNT,
            WRITE_FILE_RECORD_MAX_BYTE_COUNT,
        )?;
        EncodeError::check_byte_count(usize::from(self.byte_count), self.sub_requests.len())?;
        validate_write_file_sub_requests_encode(self.sub_requests)?;
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
