//! Spec V1.1b3 §6.4 — FC 04 (0x04) Read Input Registers conformance tests.

use modbus_codec::{decode_request, decode_response, DecodeError};
use modbus_codec::request::{Encode, ReadInputRegistersRequest};
use modbus_types::{Address, Quantity};

// Spec example p.16: read input register 9 → value 10 (0x000A)
const SPEC_REQUEST: &[u8] = &[0x04, 0x00, 0x08, 0x00, 0x01];
const SPEC_RESPONSE: &[u8] = &[0x04, 0x02, 0x00, 0x0A];

#[test]
fn spec_6_4_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        modbus_codec::RequestPdu::ReadInputRegisters(r) => {
            assert_eq!(r.address, Address(0x0008)); // register 9 (0-indexed = 8)
            assert_eq!(r.quantity, Quantity(1));
        }
        other => panic!("expected ReadInputRegisters, got {other:?}"),
    }
}

#[test]
fn spec_6_4_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        modbus_codec::ResponsePdu::ReadInputRegisters(r) => {
            assert_eq!(r.byte_count, 2);
            assert_eq!(r.register(0), 10); // 0x000A = 10 decimal
            assert_eq!(r.count(), 1);
        }
        other => panic!("expected ReadInputRegisters response, got {other:?}"),
    }
}

#[test]
fn encode_round_trip() {
    let req = ReadInputRegistersRequest {
        address: Address(0x0008),
        quantity: Quantity(1),
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn quantity_boundaries() {
    let max = [0x04, 0x00, 0x00, 0x00, 0x7D]; // 125
    assert!(decode_request(&max).is_ok());
    let over = [0x04, 0x00, 0x00, 0x00, 0x7E]; // 126
    assert!(matches!(decode_request(&over), Err(DecodeError::QuantityOutOfRange { .. })));
}

#[test]
fn truncated() {
    assert!(matches!(decode_request(&[0x04, 0x00]), Err(DecodeError::Truncated { .. })));
}
