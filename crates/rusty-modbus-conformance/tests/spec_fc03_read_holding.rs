//! Spec V1.1b3 §6.3 — FC 03 (0x03) Read Holding Registers conformance tests.

use rusty_modbus_codec::request::{Encode, ReadHoldingRegistersRequest};
use rusty_modbus_codec::{DecodeError, decode_request, decode_response};
use rusty_modbus_types::{Address, Quantity};

// Spec example p.15: read registers 108-110 → values 555, 0, 100
const SPEC_REQUEST: &[u8] = &[0x03, 0x00, 0x6B, 0x00, 0x03];
const SPEC_RESPONSE: &[u8] = &[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];

#[test]
fn spec_6_3_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::ReadHoldingRegisters(r) => {
            assert_eq!(r.address, Address(0x006B)); // register 108 (0-indexed = 107)
            assert_eq!(r.quantity, Quantity(3));
        }
        other => panic!("expected ReadHoldingRegisters, got {other:?}"),
    }
}

#[test]
fn spec_6_3_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        rusty_modbus_codec::ResponsePdu::ReadHoldingRegisters(r) => {
            assert_eq!(r.byte_count, 6);
            assert_eq!(r.register_data, &[0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]);
            // Spec p.15: register 108 = 0x022B = 555, reg 109 = 0, reg 110 = 0x0064 = 100
            assert_eq!(r.register(0), 555);
            assert_eq!(r.register(1), 0);
            assert_eq!(r.register(2), 100);
            assert_eq!(r.count(), 3);
        }
        other => panic!("expected ReadHoldingRegisters response, got {other:?}"),
    }
}

#[test]
fn encode_round_trip() {
    let req = ReadHoldingRegistersRequest {
        address: Address(0x006B),
        quantity: Quantity(3),
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn quantity_boundaries() {
    let min = [0x03, 0x00, 0x00, 0x00, 0x01]; // quantity = 1
    assert!(decode_request(&min).is_ok());
    let max = [0x03, 0x00, 0x00, 0x00, 0x7D]; // quantity = 125
    assert!(decode_request(&max).is_ok());
    let over = [0x03, 0x00, 0x00, 0x00, 0x7E]; // quantity = 126
    assert!(matches!(
        decode_request(&over),
        Err(DecodeError::QuantityOutOfRange { quantity: 126 })
    ));
    let zero = [0x03, 0x00, 0x00, 0x00, 0x00]; // quantity = 0
    assert!(matches!(
        decode_request(&zero),
        Err(DecodeError::QuantityOutOfRange { quantity: 0 })
    ));
}

#[test]
fn truncated_request() {
    assert!(matches!(
        decode_request(&[0x03, 0x00, 0x6B]),
        Err(DecodeError::Truncated { .. })
    ));
}

#[test]
fn response_byte_count_mismatch() {
    // byte_count says 6 but only 4 data bytes follow
    let bad = [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00];
    assert!(matches!(
        decode_response(&bad),
        Err(DecodeError::ByteCountMismatch { .. })
    ));
}

#[test]
fn register_iterator() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        rusty_modbus_codec::ResponsePdu::ReadHoldingRegisters(r) => {
            let regs: Vec<u16> = r.registers().collect();
            assert_eq!(regs, vec![555, 0, 100]);
        }
        _ => panic!("wrong variant"),
    }
}
