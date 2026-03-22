//! Spec V1.1b3 §6.8 — FC 08 (0x08) Diagnostics conformance tests.

use modbus_codec::{decode_request, decode_response};
use modbus_types::DiagnosticSubFunction;

// Spec example p.24: Return Query Data, sub-function 0x0000, data 0xA537
const SPEC_REQUEST: &[u8] = &[0x08, 0x00, 0x00, 0xA5, 0x37];
const SPEC_RESPONSE: &[u8] = &[0x08, 0x00, 0x00, 0xA5, 0x37];

#[test]
fn spec_6_8_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        modbus_codec::RequestPdu::Diagnostics(r) => {
            assert_eq!(r.sub_function, DiagnosticSubFunction::ReturnQueryData);
            assert_eq!(r.data, &[0xA5, 0x37]);
        }
        other => panic!("expected Diagnostics, got {other:?}"),
    }
}

#[test]
fn spec_6_8_response_is_echo() {
    // §6.8: "The normal response to the Return Query Data request is to
    // loopback the same data."
    match decode_response(SPEC_RESPONSE).unwrap() {
        modbus_codec::ResponsePdu::Diagnostics(r) => {
            assert_eq!(r.sub_function, DiagnosticSubFunction::ReturnQueryData);
            assert_eq!(r.data, &[0xA5, 0x37]);
        }
        other => panic!("expected Diagnostics response, got {other:?}"),
    }
    assert_eq!(SPEC_REQUEST, SPEC_RESPONSE);
}

#[test]
fn truncated() {
    // FC08 needs at least sub-function (2 bytes)
    assert!(matches!(decode_request(&[0x08, 0x00]), Err(modbus_codec::DecodeError::Truncated { .. })));
}
