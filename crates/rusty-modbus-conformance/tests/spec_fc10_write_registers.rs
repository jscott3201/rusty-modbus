//! Spec V1.1b3 §6.12 — FC 10 (16) Write Multiple Registers conformance tests.

use rusty_modbus_codec::{decode_request, decode_response, DecodeError};
use rusty_modbus_codec::request::Encode;
use rusty_modbus_types::{Address, Quantity};

// Spec example p.30: write 2 registers starting at register 2
// Values: 0x000A (10) and 0x0102 (258)
const SPEC_REQUEST: &[u8] = &[0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02];
const SPEC_RESPONSE: &[u8] = &[0x10, 0x00, 0x01, 0x00, 0x02];

#[test]
fn spec_6_12_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::WriteMultipleRegisters(r) => {
            assert_eq!(r.address, Address(0x0001));
            assert_eq!(r.quantity, Quantity(2));
            assert_eq!(r.byte_count, 4);
            assert_eq!(r.register_values, &[0x00, 0x0A, 0x01, 0x02]);
        }
        other => panic!("expected WriteMultipleRegisters, got {other:?}"),
    }
}

#[test]
fn spec_6_12_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        rusty_modbus_codec::ResponsePdu::WriteMultipleRegisters(r) => {
            assert_eq!(r.address, Address(0x0001));
            assert_eq!(r.quantity, Quantity(2));
        }
        other => panic!("expected WriteMultipleRegisters response, got {other:?}"),
    }
}

#[test]
fn encode_round_trip() {
    let values = [0x00, 0x0A, 0x01, 0x02];
    let req = rusty_modbus_codec::request::WriteMultipleRegistersRequest {
        address: Address(0x0001),
        quantity: Quantity(2),
        byte_count: 4,
        register_values: &values,
    };
    let mut buf = [0u8; 16];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn quantity_boundaries() {
    // Min = 1 register (2 bytes)
    let min = [0x10, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x01];
    assert!(decode_request(&min).is_ok());

    // Zero rejected
    let zero = [0x10, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert!(matches!(decode_request(&zero), Err(DecodeError::QuantityOutOfRange { quantity: 0 })));
}

#[test]
fn truncated() {
    assert!(matches!(decode_request(&[0x10, 0x00, 0x01]), Err(DecodeError::Truncated { .. })));
}
