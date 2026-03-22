//! Spec V1.1b3 §4.5 Figure 9 — Validation order conformance.
//!
//! Per the spec state diagrams (Figures 11-28), data value/quantity checks
//! (ExceptionCode 0x03) MUST be performed before address bounds checks
//! (ExceptionCode 0x02). When both fail, the returned exception code must
//! be 0x03, not 0x02.

use modbus_codec::validate::*;
use modbus_types::ExceptionCode;

// ── When both quantity AND address fail, quantity wins (0x03) ──────

#[test]
fn spec_figure_11_fc01_quantity_before_address() {
    // quantity 5000 > 2000 AND address 0xFFFF + 5000 overflows → must be 0x03
    assert_eq!(
        validate_read_coils(0xFFFF, 5000),
        Err(ExceptionCode::IllegalDataValue)
    );
}

#[test]
fn spec_figure_12_fc02_quantity_before_address() {
    assert_eq!(
        validate_read_discrete_inputs(0xFFFF, 5000),
        Err(ExceptionCode::IllegalDataValue)
    );
}

#[test]
fn spec_figure_13_fc03_quantity_before_address() {
    // quantity 200 > 125 AND address 0xFFFF + 200 overflows → must be 0x03
    assert_eq!(
        validate_read_registers(0xFFFF, 200),
        Err(ExceptionCode::IllegalDataValue)
    );
}

#[test]
fn spec_figure_21_fc0f_quantity_before_address() {
    // quantity 5000 > 1968 AND address overflow → must be 0x03
    assert_eq!(
        validate_write_coils(0xFFFF, 5000, 0),
        Err(ExceptionCode::IllegalDataValue)
    );
}

#[test]
fn spec_figure_22_fc10_quantity_before_address() {
    // quantity 200 > 123 AND address overflow → must be 0x03
    assert_eq!(
        validate_write_registers(0xFFFF, 200, 0),
        Err(ExceptionCode::IllegalDataValue)
    );
}

#[test]
fn spec_figure_27_fc17_quantity_before_address() {
    // read_quantity 200 > 125 AND address overflow → must be 0x03
    assert_eq!(
        validate_read_write_registers(0xFFFF, 200, 0, 1, 2),
        Err(ExceptionCode::IllegalDataValue)
    );
}

// ── When only address fails (valid quantity), correct 0x02 ────────

#[test]
fn fc01_address_only_failure() {
    // quantity 10 (valid) but address 0xFFF8 + 10 = 0x10002 overflows
    assert_eq!(
        validate_read_coils(0xFFF8, 10),
        Err(ExceptionCode::IllegalDataAddress)
    );
}

#[test]
fn fc03_address_only_failure() {
    assert_eq!(
        validate_read_registers(0xFFFF, 2),
        Err(ExceptionCode::IllegalDataAddress)
    );
}

#[test]
fn fc10_address_only_failure() {
    assert_eq!(
        validate_write_registers(0xFFFF, 2, 4),
        Err(ExceptionCode::IllegalDataAddress)
    );
}

// ── When only quantity fails (valid address), correct 0x03 ────────

#[test]
fn fc01_quantity_only_failure() {
    assert_eq!(
        validate_read_coils(0, 0),
        Err(ExceptionCode::IllegalDataValue)
    );
    assert_eq!(
        validate_read_coils(0, 2001),
        Err(ExceptionCode::IllegalDataValue)
    );
}

#[test]
fn fc03_quantity_only_failure() {
    assert_eq!(
        validate_read_registers(0, 126),
        Err(ExceptionCode::IllegalDataValue)
    );
}

// ── FC 05 coil value validation (§6.5 Figure 15) ──────────────────

#[test]
fn spec_figure_15_fc05_value_before_address() {
    // Value 0x0001 is invalid per §6.5
    assert_eq!(
        validate_write_single_coil(0x0001),
        Err(ExceptionCode::IllegalDataValue)
    );
}

#[test]
fn spec_figure_15_fc05_valid_values() {
    assert!(validate_write_single_coil(0xFF00).is_ok());
    assert!(validate_write_single_coil(0x0000).is_ok());
}

// ── FC 16 mask write address validation (§6.16 Figure 26) ─────────

#[test]
fn spec_figure_26_fc16_address_validation() {
    assert!(validate_mask_write_address(0x0000).is_ok());
    assert!(validate_mask_write_address(0xFFFF).is_ok()); // single register, max addr valid
}
