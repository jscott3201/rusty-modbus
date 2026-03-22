//! TCP Guide §3.1.3 — MBAP header wire format verification.

use rusty_modbus_types::{MbapHeader, MBAP_HEADER_LEN};
use zerocopy::IntoBytes;

#[test]
fn mbap_header_new_sets_fields() {
    let h = MbapHeader::new(0x1234, 0x01, 6);
    assert_eq!(h.transaction_id.get(), 0x1234);
    assert_eq!(h.protocol_id.get(), 0x0000);
    // length field = pdu_length + 1 (for unit_id byte)
    assert_eq!(h.length.get(), 7);
    assert_eq!(h.unit_id, 0x01);
}

#[test]
fn mbap_header_pdu_length() {
    let h = MbapHeader::new(0, 0xFF, 6);
    // length field = 7, pdu_length = 7 - 1 = 6
    assert_eq!(h.pdu_length(), 6);
}

#[test]
fn mbap_header_adu_length() {
    let h = MbapHeader::new(0, 0xFF, 6);
    // adu_length = MBAP_HEADER_LEN(7) + length_field(7) = 14
    assert_eq!(h.adu_length(), MBAP_HEADER_LEN + 7);
}

#[test]
fn mbap_header_wire_format_is_big_endian() {
    let h = MbapHeader::new(0x0102, 0xFF, 5);
    let bytes = h.as_bytes();
    assert_eq!(bytes.len(), 7);
    // Transaction ID: 0x0102 in BE
    assert_eq!(bytes[0], 0x01);
    assert_eq!(bytes[1], 0x02);
    // Protocol ID: 0x0000
    assert_eq!(bytes[2], 0x00);
    assert_eq!(bytes[3], 0x00);
    // Length: pdu_length(5) + 1 = 6 = 0x0006 in BE
    assert_eq!(bytes[4], 0x00);
    assert_eq!(bytes[5], 0x06);
    // Unit ID
    assert_eq!(bytes[6], 0xFF);
}

#[test]
fn mbap_header_round_trip_through_zerocopy() {
    let original = MbapHeader::new(42, 0x01, 10);
    let bytes = original.as_bytes();

    let (parsed, _rest) = zerocopy::Ref::<_, MbapHeader>::from_prefix(bytes).unwrap();
    assert_eq!(parsed.transaction_id.get(), 42);
    assert_eq!(parsed.protocol_id.get(), 0);
    assert_eq!(parsed.unit_id, 0x01);
    assert_eq!(parsed.pdu_length(), 10);
}

#[test]
fn mbap_header_pdu_length_saturates_at_zero() {
    // length = pdu_length + 1, so pdu_length(0) means length = 1, pdu = 0
    let h = MbapHeader::new(0, 0, 0);
    assert_eq!(h.pdu_length(), 0);
}

#[test]
fn mbap_header_protocol_id_always_zero() {
    // Per TCP Guide §3.1.3, protocol ID is always 0x0000 for Modbus
    let h = MbapHeader::new(0xFFFF, 0xFF, 253);
    assert_eq!(h.protocol_id.get(), 0x0000);
}
