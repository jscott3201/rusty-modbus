//! Spec V1.1b3 §6.10 — FC 0C (12) Get Comm Event Log conformance tests.

use modbus_codec::{decode_request, decode_response};

const SPEC_REQUEST: &[u8] = &[0x0C];
// Spec example p.27: status=0x0000, event_count=0x0108(264),
// message_count=0x0121(289), events=[0x20, 0x00]
const SPEC_RESPONSE: &[u8] = &[0x0C, 0x08, 0x00, 0x00, 0x01, 0x08, 0x01, 0x21, 0x20, 0x00];

#[test]
fn spec_6_10_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        modbus_codec::RequestPdu::GetCommEventLog => {}
        other => panic!("expected GetCommEventLog, got {other:?}"),
    }
}

#[test]
fn spec_6_10_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        modbus_codec::ResponsePdu::GetCommEventLog(r) => {
            assert_eq!(r.byte_count, 0x08);
            assert_eq!(r.status, 0x0000);
            assert_eq!(r.event_count, 0x0108); // 264
            assert_eq!(r.message_count, 0x0121); // 289
            assert_eq!(r.events, &[0x20, 0x00]);
        }
        other => panic!("expected GetCommEventLog response, got {other:?}"),
    }
}
