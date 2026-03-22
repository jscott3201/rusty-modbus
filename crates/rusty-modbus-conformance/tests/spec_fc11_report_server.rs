//! Spec V1.1b3 §6.13 — FC 11 (17) Report Server ID conformance tests.

use rusty_modbus_codec::{decode_request, decode_response};

const SPEC_REQUEST: &[u8] = &[0x11];

#[test]
fn spec_6_13_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::ReportServerId => {}
        other => panic!("expected ReportServerId, got {other:?}"),
    }
}

#[test]
fn response_decode_with_data() {
    // Device-specific: byte_count=2, data=[0x01, 0xFF] (server ID=1, run indicator ON)
    let resp = [0x11, 0x02, 0x01, 0xFF];
    match decode_response(&resp).unwrap() {
        rusty_modbus_codec::ResponsePdu::ReportServerId(r) => {
            assert_eq!(r.byte_count, 2);
            assert_eq!(r.data, &[0x01, 0xFF]);
        }
        other => panic!("expected ReportServerId response, got {other:?}"),
    }
}
