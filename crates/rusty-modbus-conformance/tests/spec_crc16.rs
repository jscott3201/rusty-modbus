//! CRC-16/Modbus conformance tests.
//!
//! Verifies the CRC-16 implementation against known test vectors and the
//! Modbus RTU wire format specification (little-endian CRC storage).

use rusty_modbus_frame::crc::{crc16, verify_crc};

// ── Algorithm specification ────────────────────────────────────────
// Polynomial: 0xA001 (reflected 0x8005)
// Init: 0xFFFF
// Wire format: little-endian (low byte first)

#[test]
fn crc16_init_value() {
    // Empty data → init value 0xFFFF (no XOR-out for Modbus CRC)
    assert_eq!(crc16(&[]), 0xFFFF);
}

#[test]
fn crc16_ascii_test_vector() {
    // Standard CRC-16/Modbus test: "123456789" → 0x4B37
    assert_eq!(crc16(b"123456789"), 0x4B37);
}

#[test]
fn crc16_modbus_rtu_request() {
    // Slave 0x01, FC 0x03, start 0x0000, qty 0x000A
    // Expected CRC: 0xCDC5 (wire: [0xC5, 0xCD])
    let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
    assert_eq!(crc16(&data), 0xCDC5);
}

#[test]
fn crc16_wire_format_little_endian() {
    // CRC is stored LE on wire: low byte first
    let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
    let crc = crc16(&data);
    let le_bytes = crc.to_le_bytes();
    assert_eq!(le_bytes, [0xC5, 0xCD]);
}

#[test]
fn verify_crc_valid_frame() {
    let data = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A];
    let frame = [&data[..], &[0xC5, 0xCD]].concat();
    assert!(verify_crc(&frame));
}

#[test]
fn verify_crc_invalid_frame() {
    let frame = [0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00];
    assert!(!verify_crc(&frame));
}

#[test]
fn verify_crc_too_short() {
    assert!(!verify_crc(&[]));
    assert!(!verify_crc(&[0x01]));
    assert!(!verify_crc(&[0x01, 0x03])); // need at least 3 bytes
}

#[test]
fn crc16_single_byte() {
    let crc = crc16(&[0x00]);
    assert_ne!(crc, 0xFFFF); // should differ from init
    assert_ne!(crc, 0x0000);
}

#[test]
fn crc16_fc01_example() {
    // FC01 Read Coils: slave 1, FC 0x01, addr 0x0013, qty 0x0013
    let data = [0x01, 0x01, 0x00, 0x13, 0x00, 0x13];
    let crc = crc16(&data);
    // Verify via round-trip
    let frame = [&data[..], &crc.to_le_bytes()].concat();
    assert!(verify_crc(&frame));
}
