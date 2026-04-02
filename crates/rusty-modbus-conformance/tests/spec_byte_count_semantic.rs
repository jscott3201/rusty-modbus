//! Byte count semantic validation conformance tests.
//!
//! Per spec state diagrams (Figures 21, 22, 27):
//! - FC 0F: byte_count MUST equal ceil(quantity / 8)
//! - FC 10: byte_count MUST equal quantity × 2
//! - FC 17: write_byte_count MUST equal write_quantity × 2
//!
//! The codec currently only checks byte_count == remaining_data_length (wire check)
//! but not byte_count == f(quantity) (semantic check). These tests verify the semantic
//! constraint is enforced.

use rusty_modbus_codec::decode_request;

// ── FC 0F: byte_count must equal ceil(quantity / 8) ───────────────

#[test]
fn fc0f_byte_count_matches_quantity() {
    // quantity=8 coils → byte_count should be ceil(8/8) = 1
    // Wire-valid: byte_count=1, 1 data byte → passes both checks
    let valid = [0x0F, 0x00, 0x00, 0x00, 0x08, 0x01, 0xFF];
    assert!(decode_request(&valid).is_ok());
}

#[test]
fn fc0f_byte_count_semantic_mismatch_rejected() {
    // quantity=8 coils → expected byte_count = ceil(8/8) = 1
    // But byte_count=2 with 2 data bytes → wire-valid but semantically wrong
    // Spec Figure 21: "Byte Count = N*" where N*=ceil(quantity/8)
    let bad = [0x0F, 0x00, 0x00, 0x00, 0x08, 0x02, 0xFF, 0x00];
    let result = decode_request(&bad);
    assert!(
        result.is_err(),
        "FC0F: byte_count=2 for quantity=8 should be rejected (expected 1). Got: {result:?}"
    );
}

#[test]
fn fc0f_byte_count_semantic_9_coils_needs_2_bytes() {
    // quantity=9 → byte_count should be ceil(9/8) = 2
    let valid = [0x0F, 0x00, 0x00, 0x00, 0x09, 0x02, 0xFF, 0x01];
    assert!(decode_request(&valid).is_ok());

    // byte_count=1 for 9 coils → wrong
    let bad = [0x0F, 0x00, 0x00, 0x00, 0x09, 0x01, 0xFF];
    assert!(
        decode_request(&bad).is_err(),
        "FC0F: byte_count=1 for quantity=9 should be rejected (expected 2)"
    );
}

// ── FC 10: byte_count must equal quantity × 2 ─────────────────────

#[test]
fn fc10_byte_count_matches_quantity() {
    // quantity=2 → byte_count should be 2×2 = 4
    let valid = [0x10, 0x00, 0x00, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02];
    assert!(decode_request(&valid).is_ok());
}

#[test]
fn fc10_byte_count_semantic_mismatch_rejected() {
    // quantity=2 → expected byte_count = 4
    // But byte_count=6 with 6 data bytes → wire-valid but semantically wrong
    // Spec Figure 22: "Byte Count == Quantity of Registers x 2"
    let bad = [
        0x10, 0x00, 0x00, 0x00, 0x02, 0x06, 0x00, 0x0A, 0x01, 0x02, 0x03, 0x04,
    ];
    let result = decode_request(&bad);
    assert!(
        result.is_err(),
        "FC10: byte_count=6 for quantity=2 should be rejected (expected 4). Got: {result:?}"
    );
}

// ── FC 17: write_byte_count must equal write_quantity × 2 ─────────

#[test]
fn fc17_write_byte_count_matches_quantity() {
    // write_quantity=3 → write_byte_count should be 3×2 = 6
    let valid = [
        0x17, 0x00, 0x00, 0x00, 0x01, // read addr=0, qty=1
        0x00, 0x00, 0x00, 0x03, // write addr=0, qty=3
        0x06, // write_byte_count=6
        0x00, 0x01, 0x00, 0x02, 0x00, 0x03,
    ];
    assert!(decode_request(&valid).is_ok());
}

#[test]
fn fc17_write_byte_count_semantic_mismatch_rejected() {
    // write_quantity=2 → expected write_byte_count = 4
    // But write_byte_count=6 with 6 data bytes → wire-valid but semantically wrong
    // Spec Figure 27: "Byte Count == Quantity of Write x 2"
    let bad = [
        0x17, 0x00, 0x00, 0x00, 0x01, // read addr=0, qty=1
        0x00, 0x00, 0x00, 0x02, // write addr=0, qty=2
        0x06, // write_byte_count=6 (should be 4)
        0x00, 0x01, 0x00, 0x02, 0x00, 0x03,
    ];
    let result = decode_request(&bad);
    assert!(
        result.is_err(),
        "FC17: write_byte_count=6 for write_quantity=2 should be rejected (expected 4). Got: {result:?}"
    );
}
