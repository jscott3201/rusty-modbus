//! Server-side request validation helpers.
//!
//! Implements the validation state diagrams from Spec V1.1b3 §6.1–6.18.
//! These are pure functions — no I/O.
//!
//! **Validation order matters** (per §4.5 Figure 9 and per-FC state diagrams):
//! 1. Data value: quantity in range, byte count matches → `IllegalDataValue` (0x03)
//! 2. Data address: `address + quantity` must not overflow → `IllegalDataAddress` (0x02)

use rusty_modbus_types::{
    ExceptionCode, MAX_READ_COILS, MAX_READ_DISCRETE_INPUTS, MAX_READ_REGISTERS,
    MAX_RW_READ_REGISTERS, MAX_RW_WRITE_REGISTERS, MAX_WRITE_COILS, MAX_WRITE_REGISTERS,
};

/// Validate a read coils (FC 01) request.
///
/// Per spec §6.1 Figure 11 state diagram.
///
/// # Errors
///
/// - `IllegalDataValue` if `quantity` is 0 or exceeds 2000
/// - `IllegalDataAddress` if `address + quantity > 0xFFFF`
pub fn validate_read_coils(address: u16, quantity: u16) -> Result<(), ExceptionCode> {
    if quantity == 0 || quantity > MAX_READ_COILS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    validate_address_range(address, quantity)?;
    Ok(())
}

/// Validate a read discrete inputs (FC 02) request.
///
/// Per spec §6.2 Figure 12 state diagram.
///
/// # Errors
///
/// - `IllegalDataValue` if `quantity` is 0 or exceeds 2000
/// - `IllegalDataAddress` if `address + quantity > 0xFFFF`
pub fn validate_read_discrete_inputs(address: u16, quantity: u16) -> Result<(), ExceptionCode> {
    if quantity == 0 || quantity > MAX_READ_DISCRETE_INPUTS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    validate_address_range(address, quantity)?;
    Ok(())
}

/// Validate a read holding/input registers (FC 03/04) request.
///
/// Per spec §6.3 Figure 13 / §6.4 Figure 14 state diagrams.
///
/// # Errors
///
/// - `IllegalDataValue` if `quantity` is 0 or exceeds 125
/// - `IllegalDataAddress` if `address + quantity > 0xFFFF`
pub fn validate_read_registers(address: u16, quantity: u16) -> Result<(), ExceptionCode> {
    if quantity == 0 || quantity > MAX_READ_REGISTERS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    validate_address_range(address, quantity)?;
    Ok(())
}

/// Validate a write single coil (FC 05) request.
///
/// Per spec §6.5 Figure 15 state diagram — value must be 0x0000 or 0xFF00.
///
/// # Errors
///
/// - `IllegalDataValue` if `coil_value` is not 0x0000 or 0xFF00
pub fn validate_write_single_coil(coil_value: u16) -> Result<(), ExceptionCode> {
    if coil_value != 0x0000 && coil_value != 0xFF00 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    Ok(())
}

/// Validate a write multiple coils (FC 0F) request.
///
/// Per spec §6.11 Figure 21 state diagram.
///
/// # Errors
///
/// - `IllegalDataValue` if quantity is 0 or exceeds 1968, or byte count doesn't match
/// - `IllegalDataAddress` if `address + quantity > 0xFFFF`
pub fn validate_write_coils(
    address: u16,
    quantity: u16,
    byte_count: u8,
) -> Result<(), ExceptionCode> {
    if quantity == 0 || quantity > MAX_WRITE_COILS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let expected_bytes = quantity.div_ceil(8);
    if u16::from(byte_count) != expected_bytes {
        return Err(ExceptionCode::IllegalDataValue);
    }
    validate_address_range(address, quantity)?;
    Ok(())
}

/// Validate a write multiple registers (FC 10) request.
///
/// Per spec §6.12 Figure 22 state diagram.
///
/// # Errors
///
/// - `IllegalDataValue` if quantity is 0 or exceeds 123, or byte count doesn't match
/// - `IllegalDataAddress` if `address + quantity > 0xFFFF`
pub fn validate_write_registers(
    address: u16,
    quantity: u16,
    byte_count: u8,
) -> Result<(), ExceptionCode> {
    if quantity == 0 || quantity > MAX_WRITE_REGISTERS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if u16::from(byte_count) != quantity * 2 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    validate_address_range(address, quantity)?;
    Ok(())
}

/// Validate a mask write register (FC 16) address.
///
/// Per spec §6.16 Figure 26 — both AND and OR mask values accept 0x0000–0xFFFF.
/// Only the address is validated at the protocol level.
///
/// # Errors
///
/// - `IllegalDataAddress` if `address` is not valid (overflows with quantity 1)
pub fn validate_mask_write_address(address: u16) -> Result<(), ExceptionCode> {
    validate_address_range(address, 1)
}

/// Validate a read/write multiple registers (FC 17) request.
///
/// Per spec §6.17 Figure 27 state diagram.
///
/// # Errors
///
/// - `IllegalDataValue` if quantities out of range or byte count doesn't match
/// - `IllegalDataAddress` if either address+quantity overflows
pub fn validate_read_write_registers(
    read_address: u16,
    read_quantity: u16,
    write_address: u16,
    write_quantity: u16,
    write_byte_count: u8,
) -> Result<(), ExceptionCode> {
    if read_quantity == 0 || read_quantity > MAX_RW_READ_REGISTERS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if write_quantity == 0 || write_quantity > MAX_RW_WRITE_REGISTERS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    if u16::from(write_byte_count) != write_quantity * 2 {
        return Err(ExceptionCode::IllegalDataValue);
    }
    validate_address_range(read_address, read_quantity)?;
    validate_address_range(write_address, write_quantity)?;
    Ok(())
}

/// Check that `address + quantity` doesn't overflow the 16-bit address space.
fn validate_address_range(address: u16, quantity: u16) -> Result<(), ExceptionCode> {
    if u32::from(address) + u32::from(quantity) > 0x10000 {
        return Err(ExceptionCode::IllegalDataAddress);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Read coils (FC 01) ─────────────────────────────────────────

    #[test]
    fn read_coils_valid() {
        assert!(validate_read_coils(0, 1).is_ok());
        assert!(validate_read_coils(0, 2000).is_ok());
        assert!(validate_read_coils(100, 100).is_ok());
    }

    #[test]
    fn read_coils_quantity_zero() {
        assert_eq!(validate_read_coils(0, 0), Err(ExceptionCode::IllegalDataValue));
    }

    #[test]
    fn read_coils_quantity_too_large() {
        assert_eq!(
            validate_read_coils(0, 2001),
            Err(ExceptionCode::IllegalDataValue)
        );
    }

    #[test]
    fn read_coils_address_overflow() {
        assert_eq!(
            validate_read_coils(0xFFFF, 2),
            Err(ExceptionCode::IllegalDataAddress)
        );
    }

    #[test]
    fn quantity_checked_before_address() {
        // Per spec state diagrams (V1.1b3 Figures 11-28): quantity check (0x03)
        // takes priority over address check (0x02) when both fail.
        assert_eq!(
            validate_read_coils(0xFFFF, 5000),
            Err(ExceptionCode::IllegalDataValue)
        );
    }

    // ── Read registers (FC 03/04) ──────────────────────────────────

    #[test]
    fn read_registers_valid() {
        assert!(validate_read_registers(0, 1).is_ok());
        assert!(validate_read_registers(0, 125).is_ok());
    }

    #[test]
    fn read_registers_quantity_out_of_range() {
        assert_eq!(
            validate_read_registers(0, 126),
            Err(ExceptionCode::IllegalDataValue)
        );
    }

    // ── Write single coil (FC 05) ──────────────────────────────────

    #[test]
    fn write_single_coil_valid_on() {
        assert!(validate_write_single_coil(0xFF00).is_ok());
    }

    #[test]
    fn write_single_coil_valid_off() {
        assert!(validate_write_single_coil(0x0000).is_ok());
    }

    #[test]
    fn write_single_coil_invalid_value() {
        assert_eq!(
            validate_write_single_coil(0x0001),
            Err(ExceptionCode::IllegalDataValue)
        );
        assert_eq!(
            validate_write_single_coil(0xFF01),
            Err(ExceptionCode::IllegalDataValue)
        );
    }

    // ── Write multiple coils (FC 0F) ───────────────────────────────

    #[test]
    fn write_coils_valid() {
        assert!(validate_write_coils(0, 8, 1).is_ok());
        assert!(validate_write_coils(0, 9, 2).is_ok());
        assert!(validate_write_coils(0, 1968, 246).is_ok());
    }

    #[test]
    fn write_coils_byte_count_mismatch() {
        assert_eq!(
            validate_write_coils(0, 8, 2),
            Err(ExceptionCode::IllegalDataValue)
        );
    }

    // ── Write multiple registers (FC 10) ───────────────────────────

    #[test]
    fn write_registers_valid() {
        assert!(validate_write_registers(0, 1, 2).is_ok());
        assert!(validate_write_registers(0, 123, 246).is_ok());
    }

    #[test]
    fn write_registers_byte_count_mismatch() {
        assert_eq!(
            validate_write_registers(0, 1, 3),
            Err(ExceptionCode::IllegalDataValue)
        );
    }

    // ── Mask write register (FC 16) ────────────────────────────────

    #[test]
    fn mask_write_address_valid() {
        assert!(validate_mask_write_address(0).is_ok());
        assert!(validate_mask_write_address(0xFFFE).is_ok());
    }

    #[test]
    fn mask_write_address_max_valid() {
        // 0xFFFF + 1 = 0x10000, which is exactly at the boundary (not overflow)
        assert!(validate_mask_write_address(0xFFFF).is_ok());
    }

    // ── Read/write multiple registers (FC 17) ──────────────────────

    #[test]
    fn read_write_registers_valid() {
        assert!(validate_read_write_registers(0, 1, 0, 1, 2).is_ok());
        assert!(validate_read_write_registers(0, 125, 0, 121, 242).is_ok());
    }

    #[test]
    fn read_write_registers_read_overflow() {
        assert_eq!(
            validate_read_write_registers(0xFFFF, 2, 0, 1, 2),
            Err(ExceptionCode::IllegalDataAddress)
        );
    }
}
