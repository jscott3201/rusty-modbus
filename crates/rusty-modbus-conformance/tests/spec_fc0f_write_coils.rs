//! Spec V1.1b3 §6.11 — FC 0F (15) Write Multiple Coils conformance tests.

use rusty_modbus_codec::request::Encode;
use rusty_modbus_codec::{DecodeError, decode_request, decode_response};
use rusty_modbus_types::{Address, Quantity};

// Spec example p.29: write 10 coils starting at coil 20
// Value bytes: 0xCD = 1100 1101 (coils 20-27), 0x01 = 0000 0001 (coils 28-29)
const SPEC_REQUEST: &[u8] = &[0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01];
const SPEC_RESPONSE: &[u8] = &[0x0F, 0x00, 0x13, 0x00, 0x0A];

#[test]
fn spec_6_11_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        rusty_modbus_codec::RequestPdu::WriteMultipleCoils(r) => {
            assert_eq!(r.address, Address(0x0013));
            assert_eq!(r.quantity, Quantity(0x000A)); // 10 coils
            assert_eq!(r.byte_count, 2);
            assert_eq!(r.coil_values, &[0xCD, 0x01]);
        }
        other => panic!("expected WriteMultipleCoils, got {other:?}"),
    }
}

#[test]
fn spec_6_11_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        rusty_modbus_codec::ResponsePdu::WriteMultipleCoils(r) => {
            assert_eq!(r.address, Address(0x0013));
            assert_eq!(r.quantity, Quantity(0x000A));
        }
        other => panic!("expected WriteMultipleCoils response, got {other:?}"),
    }
}

#[test]
fn encode_round_trip() {
    let req = rusty_modbus_codec::request::WriteMultipleCoilsRequest {
        address: Address(0x0013),
        quantity: Quantity(0x000A),
        byte_count: 2,
        coil_values: &[0xCD, 0x01],
    };
    let mut buf = [0u8; 16];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn quantity_boundaries() {
    // Min = 1, 1 byte
    let min = [0x0F, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01];
    assert!(decode_request(&min).is_ok());

    // Zero quantity rejected
    let zero = [0x0F, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert!(matches!(
        decode_request(&zero),
        Err(DecodeError::QuantityOutOfRange { quantity: 0 })
    ));
}

#[test]
fn truncated() {
    assert!(matches!(
        decode_request(&[0x0F, 0x00, 0x13]),
        Err(DecodeError::Truncated { .. })
    ));
}
