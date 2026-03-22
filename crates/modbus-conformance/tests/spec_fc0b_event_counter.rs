//! Spec V1.1b3 §6.9 — FC 0B (11) Get Comm Event Counter conformance tests.

use modbus_codec::{decode_request, decode_response};

const SPEC_REQUEST: &[u8] = &[0x0B];
// Spec example p.25-26: status=0xFFFF (busy), event_count=0x0108 (264)
const SPEC_RESPONSE: &[u8] = &[0x0B, 0xFF, 0xFF, 0x01, 0x08];

#[test]
fn spec_6_9_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        modbus_codec::RequestPdu::GetCommEventCounter => {}
        other => panic!("expected GetCommEventCounter, got {other:?}"),
    }
}

#[test]
fn spec_6_9_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        modbus_codec::ResponsePdu::GetCommEventCounter(r) => {
            assert_eq!(r.status, 0xFFFF);
            assert_eq!(r.event_count, 0x0108); // 264 decimal
        }
        other => panic!("expected GetCommEventCounter response, got {other:?}"),
    }
}
