//! Spec V1.1b3 §6.15 — FC 15 (21) Write File Record conformance tests.

use rusty_modbus_codec::{decode_request, decode_response};

// Spec example p.35: write 3 registers to file 4, record 7
const SPEC_REQUEST: &[u8] = &[
    0x15, 0x0D, // FC + request_data_length=13
    0x06,                   // ref_type=6
    0x00, 0x04,             // file=4
    0x00, 0x07,             // record=7
    0x00, 0x03,             // record_length=3
    0x06, 0xAF,             // register data
    0x04, 0xBE,
    0x10, 0x0D,
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
