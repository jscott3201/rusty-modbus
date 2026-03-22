//! Spec V1.1b3 §6.7 — FC 07 (0x07) Read Exception Status conformance tests.

use modbus_codec::{decode_request, decode_response};

const SPEC_REQUEST: &[u8] = &[0x07];
const SPEC_RESPONSE: &[u8] = &[0x07, 0x6D];

#[test]
fn spec_6_7_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        modbus_codec::RequestPdu::ReadExceptionStatus => {}
        other => panic!("expected ReadExceptionStatus, got {other:?}"),
    }
}

#[test]
fn spec_6_7_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        modbus_codec::ResponsePdu::ReadExceptionStatus(r) => {
            assert_eq!(r.status, 0x6D);
        }
        other => panic!("expected ReadExceptionStatus response, got {other:?}"),
    }
}
