//! Spec V1.1b3 §6.6 — FC 06 (0x06) Write Single Register conformance tests.

use modbus_codec::{decode_request, decode_response};
use modbus_codec::request::Encode;
use modbus_types::Address;

// Spec example p.19: write register 2 = 3 (address 0x0001, value 0x0003)
const SPEC_REQUEST: &[u8] = &[0x06, 0x00, 0x01, 0x00, 0x03];
const SPEC_RESPONSE: &[u8] = &[0x06, 0x00, 0x01, 0x00, 0x03];

#[test]
fn spec_6_6_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        modbus_codec::RequestPdu::WriteSingleRegister(r) => {
            assert_eq!(r.address, Address(0x0001));
            assert_eq!(r.value, 0x0003);
        }
        other => panic!("expected WriteSingleRegister, got {other:?}"),
    }
}

#[test]
fn spec_6_6_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        modbus_codec::ResponsePdu::WriteSingleRegister(r) => {
            assert_eq!(r.address, Address(0x0001));
            assert_eq!(r.value, 0x0003);
        }
        other => panic!("expected WriteSingleRegister response, got {other:?}"),
    }
}

#[test]
fn spec_6_6_response_is_echo() {
    assert_eq!(SPEC_REQUEST, SPEC_RESPONSE);
}

#[test]
fn encode_round_trip() {
    let req = modbus_codec::request::WriteSingleRegisterRequest {
        address: Address(0x0001),
        value: 0x0003,
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn full_range_register_value() {
    // §6.6: register value 0x0000 to 0xFFFF — all values valid
    let min = [0x06, 0x00, 0x00, 0x00, 0x00];
    assert!(decode_request(&min).is_ok());
    let max = [0x06, 0x00, 0x00, 0xFF, 0xFF];
    assert!(decode_request(&max).is_ok());
}

#[test]
fn truncated() {
    assert!(matches!(
        decode_request(&[0x06, 0x00]),
        Err(modbus_codec::DecodeError::Truncated { .. })
    ));
}
