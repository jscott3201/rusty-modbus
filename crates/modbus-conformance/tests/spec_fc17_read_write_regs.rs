//! Spec V1.1b3 §6.17 — FC 17 (23) Read/Write Multiple Registers conformance tests.

use modbus_codec::{decode_request, decode_response, DecodeError};
use modbus_types::{Address, Quantity};

// Spec example p.38: read 6 regs starting at 4, write 3 regs starting at 15
// Write values: 0x00FF, 0x00FF, 0x00FF
const SPEC_REQUEST: &[u8] = &[
    0x17,
    0x00, 0x03, // read address = 3 (register 4, 0-indexed)
    0x00, 0x06, // read quantity = 6
    0x00, 0x0E, // write address = 14 (register 15, 0-indexed)
    0x00, 0x03, // write quantity = 3
    0x06,       // write byte count = 6
    0x00, 0xFF, // write value 1
    0x00, 0xFF, // write value 2
    0x00, 0xFF, // write value 3
];

// Response contains the 6 read registers
const SPEC_RESPONSE: &[u8] = &[
    0x17,
    0x0C, // byte count = 12 (6 registers × 2 bytes)
    0x00, 0xFE, 0x0A, 0xCD, 0x00, 0x01, 0x00, 0x03, 0x00, 0x0D, 0x00, 0xFF,
];

#[test]
fn spec_6_17_request_decode() {
    match decode_request(SPEC_REQUEST).unwrap() {
        modbus_codec::RequestPdu::ReadWriteMultipleRegisters(r) => {
            assert_eq!(r.read_address, Address(0x0003));
            assert_eq!(r.read_quantity, Quantity(6));
            assert_eq!(r.write_address, Address(0x000E));
            assert_eq!(r.write_quantity, Quantity(3));
            assert_eq!(r.write_byte_count, 6);
            assert_eq!(r.write_register_values, &[0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF]);
        }
        other => panic!("expected ReadWriteMultipleRegisters, got {other:?}"),
    }
}

#[test]
fn spec_6_17_response_decode() {
    match decode_response(SPEC_RESPONSE).unwrap() {
        modbus_codec::ResponsePdu::ReadWriteMultipleRegisters(r) => {
            assert_eq!(r.byte_count, 12);
            assert_eq!(r.register_data.len(), 12);
            // First register = 0x00FE = 254
            assert_eq!(r.register(0), 0x00FE);
        }
        other => panic!("expected ReadWriteMultipleRegisters response, got {other:?}"),
    }
}

#[test]
fn quantity_boundaries() {
    // Read quantity max = 125 (0x007D), write quantity max = 121 (0x0079)
    // Read quantity 0 → error
    let zero_read = [
        0x17,
        0x00, 0x00, 0x00, 0x00, // read addr=0, qty=0
        0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x01, // write addr=0, qty=1, bc=2, data
    ];
    assert!(matches!(decode_request(&zero_read), Err(DecodeError::QuantityOutOfRange { .. })));
}

#[test]
fn truncated() {
    assert!(matches!(
        decode_request(&[0x17, 0x00, 0x03]),
        Err(DecodeError::Truncated { .. })
    ));
}
