//! Spec V1.1b3 §6.16 — FC 16 (22) Mask Write Register conformance tests.

use rusty_modbus_codec::{decode_request, decode_response};
use rusty_modbus_codec::request::Encode;
use rusty_modbus_types::Address;

// Spec example p.37: mask write register 5 with AND=0x00F2, OR=0x0025
// Algorithm: (Current AND And_Mask) OR (Or_Mask AND NOT(And_Mask))
// (0x0012 AND 0x00F2) OR (0x0025 AND 0x000D) = 0x0012 OR 0x0005 = 0x0017
const SPEC_REQUEST: &[u8] = &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25];
const SPEC_RESPONSE: &[u8] = &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25];

#[test]
fn spec_6_16_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::MaskWriteRegister(r) => {
            assert_eq!(r.address, Address(0x0004));
            assert_eq!(r.and_mask, 0x00F2);
            assert_eq!(r.or_mask, 0x0025);
        }
        other => panic!("expected MaskWriteRegister, got {other:?}"),
    }
}

#[test]
fn spec_6_16_response_is_echo() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        rusty_modbus_codec::ResponsePdu::MaskWriteRegister(r) => {
            assert_eq!(r.address, Address(0x0004));
            assert_eq!(r.and_mask, 0x00F2);
            assert_eq!(r.or_mask, 0x0025);
        }
        other => panic!("expected MaskWriteRegister response, got {other:?}"),
    }
    assert_eq!(SPEC_REQUEST, SPEC_RESPONSE);
}

#[test]
fn spec_6_16_mask_algorithm_verification() {
    // Spec p.36: Current=0x0012, AND=0xF2, OR=0x25
    // Result = (0x0012 AND 0x00F2) OR (0x0025 AND NOT(0x00F2))
    //        = 0x0012 OR (0x0025 AND 0xFF0D)
    //        = 0x0012 OR 0x0005
    //        = 0x0017
    let current: u16 = 0x0012;
    let and_mask: u16 = 0x00F2;
    let or_mask: u16 = 0x0025;
    let result = (current & and_mask) | (or_mask & !and_mask);
    assert_eq!(result, 0x0017);
}

#[test]
fn encode_round_trip() {
    let req = rusty_modbus_codec::request::MaskWriteRegisterRequest {
        address: Address(0x0004),
        and_mask: 0x00F2,
        or_mask: 0x0025,
    };
    let mut buf = [0u8; 7];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn truncated() {
    assert!(matches!(
        decode_request(&[0x16, 0x00, 0x04, 0x00]),
        Err(rusty_modbus_codec::DecodeError::Truncated { .. })
    ));
}
