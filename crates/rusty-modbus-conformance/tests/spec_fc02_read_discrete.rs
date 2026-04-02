//! Spec V1.1b3 §6.2 — FC 02 (0x02) Read Discrete Inputs conformance tests.

use rusty_modbus_codec::request::{Encode, ReadDiscreteInputsRequest};
use rusty_modbus_codec::{DecodeError, decode_request, decode_response};
use rusty_modbus_types::{Address, Quantity};

// Spec example: read discrete inputs 197-218 (address 0x00C4, quantity 0x0016 = 22)
const SPEC_REQUEST: &[u8] = &[0x02, 0x00, 0xC4, 0x00, 0x16];
const SPEC_RESPONSE: &[u8] = &[0x02, 0x03, 0xAC, 0xDB, 0x35];

#[test]
fn spec_6_2_request_decode() {
    let req = decode_request(SPEC_REQUEST).unwrap();
    match req {
        rusty_modbus_codec::RequestPdu::ReadDiscreteInputs(r) => {
            assert_eq!(r.address, Address(0x00C4));
            assert_eq!(r.quantity, Quantity(0x0016));
        }
        _ => panic!("expected ReadDiscreteInputs"),
    }
}

#[test]
fn spec_6_2_response_decode() {
    let resp = decode_response(SPEC_RESPONSE).unwrap();
    match resp {
        rusty_modbus_codec::ResponsePdu::ReadDiscreteInputs(r) => {
            assert_eq!(r.byte_count, 3);
            assert_eq!(r.coil_status, &[0xAC, 0xDB, 0x35]);
        }
        _ => panic!("expected ReadDiscreteInputs response"),
    }
}

#[test]
fn encode_round_trip() {
    let req = ReadDiscreteInputsRequest {
        address: Address(0x00C4),
        quantity: Quantity(0x0016),
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);
}

#[test]
fn spec_6_2_input_bit_packing() {
    // 0xAC = 1010 1100 → inputs 197(LSB)-204(MSB)
    // Input 197 (bit 0) = 0, Input 198 = 0, Input 199 = 1, Input 200 = 1
    // Input 201 = 0, Input 202 = 1, Input 203 = 0, Input 204 = 1
    let resp = decode_response(SPEC_RESPONSE).unwrap();
    match resp {
        rusty_modbus_codec::ResponsePdu::ReadDiscreteInputs(r) => {
            assert!(!r.coil(0)); // input 197 = OFF
            assert!(!r.coil(1)); // input 198 = OFF
            assert!(r.coil(2)); // input 199 = ON
            assert!(r.coil(3)); // input 200 = ON
            assert!(!r.coil(4)); // input 201 = OFF
            assert!(r.coil(5)); // input 202 = ON
            assert!(!r.coil(6)); // input 203 = OFF
            assert!(r.coil(7)); // input 204 = ON
        }
        _ => panic!("expected ReadDiscreteInputs"),
    }
}

#[test]
fn quantity_boundary() {
    // Max = 2000
    let max_pdu = [0x02, 0x00, 0x00, 0x07, 0xD0];
    assert!(decode_request(&max_pdu).is_ok());
    let over_pdu = [0x02, 0x00, 0x00, 0x07, 0xD1];
    assert!(matches!(
        decode_request(&over_pdu),
        Err(DecodeError::QuantityOutOfRange { .. })
    ));
}

#[test]
fn truncated() {
    assert!(matches!(
        decode_request(&[0x02, 0x00]),
        Err(DecodeError::Truncated { .. })
    ));
}
