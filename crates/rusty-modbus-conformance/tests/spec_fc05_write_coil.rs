//! Spec V1.1b3 §6.5 — FC 05 (0x05) Write Single Coil conformance tests.

use rusty_modbus_codec::{decode_request, decode_response, DecodeError};
use rusty_modbus_codec::request::Encode;
use rusty_modbus_types::{Address, CoilValue};

// Spec example p.18: write coil 173 ON (address 0x00AC, value 0xFF00)
const SPEC_REQUEST: &[u8] = &[0x05, 0x00, 0xAC, 0xFF, 0x00];
// Response is exact echo per spec
const SPEC_RESPONSE: &[u8] = &[0x05, 0x00, 0xAC, 0xFF, 0x00];

#[test]
fn spec_6_5_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::WriteSingleCoil(r) => {
            assert_eq!(r.address, Address(0x00AC));
            assert_eq!(r.value, CoilValue::On);
        }
        other => panic!("expected WriteSingleCoil, got {other:?}"),
    }
}

#[test]
fn spec_6_5_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        rusty_modbus_codec::ResponsePdu::WriteSingleCoil(r) => {
            assert_eq!(r.address, Address(0x00AC));
            assert_eq!(r.value, CoilValue::On);
        }
        other => panic!("expected WriteSingleCoil response, got {other:?}"),
    }
}

#[test]
fn spec_6_5_response_is_echo() {
    // §6.5: "The normal response is an echo of the request"
    assert_eq!(SPEC_REQUEST, SPEC_RESPONSE);
}

#[test]
fn write_coil_off() {
    let pdu = [0x05, 0x00, 0x00, 0x00, 0x00]; // coil 1 OFF
    match decode_request(&pdu).unwrap() {
        rusty_modbus_codec::RequestPdu::WriteSingleCoil(r) => {
            assert_eq!(r.value, CoilValue::Off);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn spec_6_5_invalid_coil_value_rejected() {
    // §6.5: "All other values are illegal and will not affect the output"
    let bad = [0x05, 0x00, 0x00, 0x00, 0x01]; // 0x0001 is invalid
    assert!(matches!(decode_request(&bad), Err(DecodeError::InvalidCoilValue(0x0001))));
}

#[test]
fn invalid_coil_value_ff01() {
    let bad = [0x05, 0x00, 0x00, 0xFF, 0x01]; // 0xFF01 is invalid
    assert!(matches!(decode_request(&bad), Err(DecodeError::InvalidCoilValue(0xFF01))));
}

#[test]
fn encode_round_trip() {
    let req = rusty_modbus_codec::request::WriteSingleCoilRequest {
        address: Address(0x00AC),
        value: CoilValue::On,
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn truncated() {
    assert!(matches!(decode_request(&[0x05, 0x00, 0xAC]), Err(DecodeError::Truncated { .. })));
}
