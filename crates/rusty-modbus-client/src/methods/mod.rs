//! Typed request methods for every Modbus function code.

use rusty_modbus_codec::{Encode, EncodeError};

use crate::error::ClientError;

mod coils;
mod device_id;
mod fifo;
mod file;
mod registers;

const MAX_WRITE_MULTIPLE_COILS: usize = 1968;
const MAX_WRITE_MULTIPLE_REGISTERS: usize = 123;
const MAX_READ_WRITE_MULTIPLE_WRITE_REGISTERS: usize = 121;
const FILE_RECORD_MIN_BYTE_COUNT: usize = 0x07;
const FILE_RECORD_MAX_BYTE_COUNT: usize = 0xF5;

fn encode_request(request: &impl Encode, buf: &mut [u8]) -> Result<usize, ClientError> {
    request.encode_into(buf).map_err(ClientError::Encode)
}

fn checked_quantity_len(len: usize, maximum: usize) -> Result<u16, ClientError> {
    let quantity = u16::try_from(len).unwrap_or(u16::MAX);
    if len == 0 || len > maximum {
        Err(EncodeError::QuantityOutOfRange { quantity }.into())
    } else {
        Ok(quantity)
    }
}

fn checked_byte_count_value(len: usize, minimum: usize, maximum: usize) -> Result<u8, ClientError> {
    if len < minimum || len > maximum {
        Err(EncodeError::ByteCountOutOfRange {
            count: len,
            minimum,
            maximum,
        }
        .into())
    } else {
        u8::try_from(len).map_err(|_| {
            ClientError::Encode(EncodeError::ByteCountOutOfRange {
                count: len,
                minimum,
                maximum,
            })
        })
    }
}

fn checked_byte_count_len(len: usize) -> Result<u8, ClientError> {
    checked_byte_count_value(len, 0, usize::from(u8::MAX))
}

fn checked_file_record_byte_count(len: usize) -> Result<u8, ClientError> {
    checked_byte_count_value(len, FILE_RECORD_MIN_BYTE_COUNT, FILE_RECORD_MAX_BYTE_COUNT)
}
