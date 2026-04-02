//! Spec V1.1b3 §6.1 — FC 01 (0x01) Read Coils conformance tests.

use rusty_modbus_codec::request::{Encode, ReadCoilsRequest};
use rusty_modbus_codec::{DecodeError, decode_request, decode_response};
use rusty_modbus_types::{Address, Quantity};

// Spec example: read discrete outputs 20-38 (address 0x0013, quantity 0x0013 = 19)
const SPEC_REQUEST: &[u8] = &[0x01, 0x00, 0x13, 0x00, 0x13];
const SPEC_RESPONSE: &[u8] = &[0x01, 0x03, 0xCD, 0x6B, 0x05];

// ── A. Byte-exact wire format ──────────────────────────────────────

#[test]
fn spec_6_1_request_decode() {
    let req = decode_request(SPEC_REQUEST).unwrap();
    match req {
        rusty_modbus_codec::RequestPdu::ReadCoils(r) => {
            assert_eq!(r.address, Address(0x0013));
            assert_eq!(r.quantity, Quantity(0x0013));
        }
        _ => panic!("expected ReadCoils, got {req:?}"),
    }
}

#[test]
fn spec_6_1_response_decode() {
    let resp = decode_response(SPEC_RESPONSE).unwrap();
    match resp {
        rusty_modbus_codec::ResponsePdu::ReadCoils(r) => {
            assert_eq!(r.byte_count, 3);
            assert_eq!(r.coil_status, &[0xCD, 0x6B, 0x05]);
        }
        _ => panic!("expected ReadCoils response, got {resp:?}"),
    }
}

// ── B. Encode round-trip ───────────────────────────────────────────

#[test]
fn encode_round_trip() {
    let req = ReadCoilsRequest {
        address: Address(0x0013),
        quantity: Quantity(0x0013),
    };
    let mut buf = [0u8; 5];
    let len = req.encode_into(&mut buf).unwrap();
    assert_eq!(&buf[..len], SPEC_REQUEST);

    // Decode back
    let decoded = decode_request(&buf[..len]).unwrap();
    match decoded {
        rusty_modbus_codec::RequestPdu::ReadCoils(r) => {
            assert_eq!(r.address, Address(0x0013));
            assert_eq!(r.quantity, Quantity(0x0013));
        }
        _ => panic!("round-trip failed"),
    }
}

// ── C. Boundary values ─────────────────────────────────────────────

#[test]
fn quantity_min_valid() {
    let pdu = [0x01, 0x00, 0x00, 0x00, 0x01]; // quantity = 1
    assert!(decode_request(&pdu).is_ok());
}

#[test]
fn quantity_max_valid() {
    let pdu = [0x01, 0x00, 0x00, 0x07, 0xD0]; // quantity = 2000
    assert!(decode_request(&pdu).is_ok());
}

#[test]
fn quantity_zero_rejected() {
    let pdu = [0x01, 0x00, 0x00, 0x00, 0x00]; // quantity = 0
    assert!(matches!(
        decode_request(&pdu),
        Err(DecodeError::QuantityOutOfRange { quantity: 0 })
    ));
}

#[test]
fn quantity_above_max_rejected() {
    let pdu = [0x01, 0x00, 0x00, 0x07, 0xD1]; // quantity = 2001
    assert!(matches!(
        decode_request(&pdu),
        Err(DecodeError::QuantityOutOfRange { quantity: 2001 })
    ));
}

// ── D. Truncation ──────────────────────────────────────────────────

#[test]
fn truncated_empty() {
    assert!(matches!(
        decode_request(&[]),
        Err(DecodeError::Truncated { .. })
    ));
}

#[test]
fn truncated_fc_only() {
    assert!(matches!(
        decode_request(&[0x01]),
        Err(DecodeError::Truncated { .. })
    ));
}

#[test]
fn truncated_partial_address() {
    assert!(matches!(
        decode_request(&[0x01, 0x00, 0x13]),
        Err(DecodeError::Truncated { .. })
    ));
}

// ── E. Coil bit verification ───────────────────────────────────────

#[test]
fn spec_6_1_coil_bit_packing() {
    // From spec p.12: outputs 20-38
    // 0xCD = 1100 1101 → outputs 20(LSB)-27(MSB)
    // Output 20 (bit 0 of byte 0) = 1 (ON)
    // Output 21 (bit 1) = 0 (OFF)
    // Output 22 (bit 2) = 1 (ON)
    // Output 23 (bit 3) = 1 (ON)
    // Output 24 (bit 4) = 0 (OFF)
    // Output 25 (bit 5) = 0 (OFF)
    // Output 26 (bit 6) = 1 (ON)
    // Output 27 (bit 7) = 1 (ON)
    let resp = decode_response(SPEC_RESPONSE).unwrap();
    match resp {
        rusty_modbus_codec::ResponsePdu::ReadCoils(r) => {
            assert!(r.coil(0)); // output 20 = ON
            assert!(!r.coil(1)); // output 21 = OFF
            assert!(r.coil(2)); // output 22 = ON
            assert!(r.coil(3)); // output 23 = ON
            assert!(!r.coil(4)); // output 24 = OFF
            assert!(!r.coil(5)); // output 25 = OFF
            assert!(r.coil(6)); // output 26 = ON
            assert!(r.coil(7)); // output 27 = ON

            // 0x6B = 0110 1011 → outputs 28-35
            assert!(r.coil(8)); // output 28 = ON
            assert!(r.coil(9)); // output 29 = ON
            assert!(!r.coil(10)); // output 30 = OFF
            assert!(r.coil(11)); // output 31 = ON

            // 0x05 = 0000 0101 → outputs 36-38 (only 3 bits used)
            assert!(r.coil(16)); // output 36 = ON
            assert!(!r.coil(17)); // output 37 = OFF
            assert!(r.coil(18)); // output 38 = ON
        }
        _ => panic!("expected ReadCoils"),
    }
}
