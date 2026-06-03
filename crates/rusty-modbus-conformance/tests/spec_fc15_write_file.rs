//! Spec V1.1b3 §6.15 — FC 15 (21) Write File Record conformance tests.

use rusty_modbus_codec::{decode_request, decode_response};

// Spec example p.35: write 3 registers to file 4, record 7
const SPEC_REQUEST: &[u8] = &[
    0x15, 0x0D, // FC + request_data_length=13
    0x06, // ref_type=6
    0x00, 0x04, // file=4
    0x00, 0x07, // record=7
    0x00, 0x03, // record_length=3
    0x06, 0xAF, // register data
    0x04, 0xBE, 0x10, 0x0D,
];

#[test]
fn spec_6_15_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::WriteFileRecord(r) => {
            assert_eq!(r.byte_count, 0x0D);
        }
        other => panic!("expected WriteFileRecord, got {other:?}"),
    }
}

#[test]
fn spec_6_15_response_is_echo() {
    // §6.15: "The normal response is an echo of the request."
    match decode_response(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::ResponsePdu::WriteFileRecord(r) => {
            assert_eq!(r.byte_count, 0x0D);
        }
        other => panic!("expected WriteFileRecord response, got {other:?}"),
    }
}

#[test]
fn request_byte_count_must_be_in_spec_range() {
    assert!(matches!(
        decode_request(&[0x15, 0x08, 0, 0, 0, 0, 0, 0, 0, 0]),
        Err(rusty_modbus_codec::DecodeError::ByteCountOutOfRange {
            count: 8,
            minimum: 9,
            maximum: 251,
        })
    ));

    let mut pdu = vec![0x15, 0xFC];
    pdu.extend_from_slice(&[0; 252]);
    assert!(matches!(
        decode_request(&pdu),
        Err(rusty_modbus_codec::DecodeError::PduTooLarge {
            length: 254,
            maximum: 253,
        })
    ));
}

#[test]
fn response_byte_count_must_be_in_spec_range() {
    assert!(matches!(
        decode_response(&[0x15, 0x08, 0, 0, 0, 0, 0, 0, 0, 0]),
        Err(rusty_modbus_codec::DecodeError::ByteCountOutOfRange {
            count: 8,
            minimum: 9,
            maximum: 251,
        })
    ));

    let mut pdu = vec![0x15, 0xFC];
    pdu.extend_from_slice(&[0; 252]);
    assert!(matches!(
        decode_response(&pdu),
        Err(rusty_modbus_codec::DecodeError::PduTooLarge {
            length: 254,
            maximum: 253,
        })
    ));
}

#[test]
fn request_reference_type_must_be_6() {
    assert_eq!(
        decode_request(&[0x15, 0x09, 0x07, 0, 1, 0, 0, 0, 1, 0x12, 0x34]).unwrap_err(),
        rusty_modbus_codec::DecodeError::InvalidReferenceType(0x07)
    );
}

#[test]
fn request_record_length_must_match_payload() {
    assert_eq!(
        decode_request(&[0x15, 0x09, 0x06, 0, 1, 0, 0, 0, 2, 0x12, 0x34]).unwrap_err(),
        rusty_modbus_codec::DecodeError::ByteCountMismatch {
            declared: 11,
            actual: 9,
        }
    );
}

#[test]
fn request_file_record_range_must_be_valid() {
    assert_eq!(
        decode_request(&[0x15, 0x09, 0x06, 0, 0, 0, 0, 0, 1, 0x12, 0x34]).unwrap_err(),
        rusty_modbus_codec::DecodeError::FileRecordOutOfRange {
            file_number: 0,
            record_number: 0,
            record_length: 1,
        }
    );
}

#[test]
fn response_is_validated_as_echo_payload() {
    assert_eq!(
        decode_response(&[0x15, 0x09, 0x07, 0, 1, 0, 0, 0, 1, 0x12, 0x34]).unwrap_err(),
        rusty_modbus_codec::DecodeError::InvalidReferenceType(0x07)
    );
}
