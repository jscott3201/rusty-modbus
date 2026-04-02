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
fn truncated() {
    assert!(matches!(
        decode_request(&[0x14]),
        Err(rusty_modbus_codec::DecodeError::Truncated { .. })
    ));
}
