//! Spec V1.1b3 §6.14 — FC 14 (20) Read File Record conformance tests.

use rusty_modbus_codec::{decode_request, decode_response};

// Spec example p.33: two groups of file record references
// Group 1: file 4, record 1, length 2
// Group 2: file 3, record 9, length 2
const SPEC_REQUEST: &[u8] = &[
    0x14, 0x0E, // FC + byte_count=14
    0x06, 0x00, 0x04, 0x00, 0x01, 0x00,
    0x02, // sub-req 1: ref_type=6, file=4, record=1, len=2
    0x06, 0x00, 0x03, 0x00, 0x09, 0x00,
    0x02, // sub-req 2: ref_type=6, file=3, record=9, len=2
];

#[test]
fn spec_6_14_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::ReadFileRecord(r) => {
            assert_eq!(r.byte_count, 0x0E); // 14 bytes of sub-requests
            assert_eq!(r.sub_requests.len(), 14);
        }
        other => panic!("expected ReadFileRecord, got {other:?}"),
    }
}

#[test]
fn spec_6_14_response_decode() {
    // Spec response p.33
    let resp: &[u8] = &[
        0x14, 0x0C, // FC + resp_data_length=12
        0x05, 0x06, 0x0D, 0xFE, 0x00,
        0x20, // sub-resp 1: len=5, ref_type=6, data=[0x0DFE, 0x0020]
        0x05, 0x06, 0x33, 0xCD, 0x00,
        0x40, // sub-resp 2: len=5, ref_type=6, data=[0x33CD, 0x0040]
    ];
    match decode_response(resp).unwrap() {
        rusty_modbus_codec::ResponsePdu::ReadFileRecord(r) => {
            assert_eq!(r.byte_count, 0x0C);
        }
        other => panic!("expected ReadFileRecord response, got {other:?}"),
    }
}

#[test]
fn response_accepts_single_register_sub_response() {
    let resp: &[u8] = &[0x14, 0x04, 0x03, 0x06, 0x12, 0x34];
    match decode_response(resp).unwrap() {
        rusty_modbus_codec::ResponsePdu::ReadFileRecord(r) => {
            assert_eq!(r.byte_count, 0x04);
            assert_eq!(r.data, &[0x03, 0x06, 0x12, 0x34]);
        }
        other => panic!("expected ReadFileRecord response, got {other:?}"),
    }
}

#[test]
fn truncated() {
    assert!(matches!(
        decode_request(&[0x14]),
        Err(rusty_modbus_codec::DecodeError::Truncated { .. })
    ));
}

#[test]
fn request_byte_count_must_be_in_spec_range() {
    assert!(matches!(
        decode_request(&[0x14, 0x06, 0, 0, 0, 0, 0, 0]),
        Err(rusty_modbus_codec::DecodeError::ByteCountOutOfRange {
            count: 6,
            minimum: 7,
            maximum: 245,
        })
    ));

    let mut pdu = vec![0x14, 0xF6];
    pdu.extend_from_slice(&[0; 246]);
    assert!(matches!(
        decode_request(&pdu),
        Err(rusty_modbus_codec::DecodeError::ByteCountOutOfRange {
            count: 246,
            minimum: 7,
            maximum: 245,
        })
    ));
}

#[test]
fn request_sub_requests_must_be_7_byte_groups() {
    assert_eq!(
        decode_request(&[0x14, 0x08, 0x06, 0, 1, 0, 0, 0, 1, 0]).unwrap_err(),
        rusty_modbus_codec::DecodeError::InvalidFileRecordLength { length: 8 }
    );
}

#[test]
fn request_reference_type_must_be_6() {
    assert_eq!(
        decode_request(&[0x14, 0x07, 0x07, 0, 1, 0, 0, 0, 1]).unwrap_err(),
        rusty_modbus_codec::DecodeError::InvalidReferenceType(0x07)
    );
}

#[test]
fn request_file_record_range_must_be_valid() {
    assert_eq!(
        decode_request(&[0x14, 0x07, 0x06, 0, 0, 0, 0, 0, 1]).unwrap_err(),
        rusty_modbus_codec::DecodeError::FileRecordOutOfRange {
            file_number: 0,
            record_number: 0,
            record_length: 1,
        }
    );
}

#[test]
fn response_byte_count_must_be_in_spec_range() {
    assert!(matches!(
        decode_response(&[0x14, 0x03, 0, 0, 0]),
        Err(rusty_modbus_codec::DecodeError::ByteCountOutOfRange {
            count: 3,
            minimum: 4,
            maximum: 250,
        })
    ));

    let mut pdu = vec![0x14, 0xFB];
    pdu.extend_from_slice(&[0; 251]);
    assert!(matches!(
        decode_response(&pdu),
        Err(rusty_modbus_codec::DecodeError::ByteCountOutOfRange {
            count: 251,
            minimum: 4,
            maximum: 250,
        })
    ));
}

#[test]
fn response_sub_responses_must_have_reference_type_6() {
    assert_eq!(
        decode_response(&[0x14, 0x04, 0x03, 0x07, 0x12, 0x34]).unwrap_err(),
        rusty_modbus_codec::DecodeError::InvalidReferenceType(0x07)
    );
}
