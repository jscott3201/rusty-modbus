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

fn validate_response_length(
    function_code: u8,
    expected: usize,
    actual: usize,
) -> Result<(), ClientError> {
    match actual.cmp(&expected) {
        core::cmp::Ordering::Less => Err(ClientError::ShortResponse { expected, actual }),
        core::cmp::Ordering::Equal => Ok(()),
        core::cmp::Ordering::Greater => Err(ClientError::UnexpectedResponseLength {
            function_code,
            expected,
            actual,
        }),
    }
}

fn validate_bit_response_shape(
    function_code: u8,
    quantity: u16,
    data: &[u8],
) -> Result<(), ClientError> {
    let expected = usize::from(quantity).div_ceil(8);
    validate_response_length(function_code, expected, data.len())?;

    let remainder = quantity % 8;
    if remainder != 0 {
        let invalid_mask = u8::MAX << remainder;
        let actual = data[expected - 1];
        if actual & invalid_mask != 0 {
            return Err(ClientError::UnexpectedResponsePadding {
                function_code,
                invalid_mask,
                actual,
            });
        }
    }

    Ok(())
}

fn validate_register_response_shape(
    function_code: u8,
    quantity: u16,
    data: &[u8],
) -> Result<(), ClientError> {
    validate_response_length(function_code, usize::from(quantity) * 2, data.len())
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

#[cfg(test)]
mod tests {
    use super::{validate_bit_response_shape, validate_register_response_shape};
    use crate::ClientError;

    #[test]
    fn bit_response_shape_covers_protocol_boundaries() {
        for quantity in [1u16, 8, 9, 1999, 2000] {
            let expected = usize::from(quantity).div_ceil(8);
            let exact = vec![0u8; expected];
            assert!(validate_bit_response_shape(0x01, quantity, &exact).is_ok());

            assert!(matches!(
                validate_bit_response_shape(0x01, quantity, &exact[..expected - 1]),
                Err(ClientError::ShortResponse {
                    expected: got_expected,
                    actual,
                }) if got_expected == expected && actual == expected - 1
            ));

            let mut overlong = exact.clone();
            overlong.push(0);
            assert!(matches!(
                validate_bit_response_shape(0x01, quantity, &overlong),
                Err(ClientError::UnexpectedResponseLength {
                    function_code: 0x01,
                    expected: got_expected,
                    actual,
                }) if got_expected == expected && actual == expected + 1
            ));

            let remainder = quantity % 8;
            if remainder == 0 {
                let mut all_bits_set = exact;
                all_bits_set[expected - 1] = u8::MAX;
                assert!(validate_bit_response_shape(0x01, quantity, &all_bits_set).is_ok());
            } else {
                let invalid_mask = u8::MAX << remainder;
                let mut invalid = exact;
                invalid[expected - 1] = invalid_mask;
                assert!(matches!(
                    validate_bit_response_shape(0x01, quantity, &invalid),
                    Err(ClientError::UnexpectedResponsePadding {
                        function_code: 0x01,
                        invalid_mask: got_mask,
                        actual,
                    }) if got_mask == invalid_mask && actual == invalid_mask
                ));
            }
        }
    }

    #[test]
    fn register_response_shape_covers_protocol_boundaries() {
        for quantity in [1u16, 125] {
            let expected = usize::from(quantity) * 2;
            let exact = vec![0u8; expected];
            assert!(validate_register_response_shape(0x03, quantity, &exact).is_ok());

            assert!(matches!(
                validate_register_response_shape(0x03, quantity, &exact[..expected - 2]),
                Err(ClientError::ShortResponse {
                    expected: got_expected,
                    actual,
                }) if got_expected == expected && actual == expected - 2
            ));

            let mut overlong = exact.clone();
            overlong.extend_from_slice(&[0, 0]);
            assert!(matches!(
                validate_register_response_shape(0x03, quantity, &overlong),
                Err(ClientError::UnexpectedResponseLength {
                    function_code: 0x03,
                    expected: got_expected,
                    actual,
                }) if got_expected == expected && actual == expected + 2
            ));

            assert!(matches!(
                validate_register_response_shape(0x03, quantity, &exact[..expected - 1]),
                Err(ClientError::ShortResponse {
                    expected: got_expected,
                    actual,
                }) if got_expected == expected && actual == expected - 1
            ));
        }
    }
}
