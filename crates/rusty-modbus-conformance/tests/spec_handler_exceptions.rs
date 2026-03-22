//! Handler-level conformance tests for exception code correctness.
//!
//! Per spec V1.1b3 §4.5 Figure 9:
//! - Unknown function code → IllegalFunction (0x01)
//! - Known FC with bad data (truncated, bad quantity, bad coil value) → IllegalDataValue (0x03)
//!
//! These tests call the handler's process_request directly with crafted PDUs.

use std::sync::Arc;
use rusty_modbus_server::handler;
use rusty_modbus_server::store::memory::{InMemoryStore, StoreConfig};
use rusty_modbus_server::DeviceIdentification;
use rusty_modbus_types::{ExceptionCode, UnitId};

fn store() -> Arc<InMemoryStore> {
    Arc::new(InMemoryStore::new(StoreConfig::default()))
}

/// Parse exception response PDU and return (fc_with_flag, exception_code)
fn parse_exception(pdu: &[u8]) -> (u8, u8) {
    assert_eq!(pdu.len(), 2, "exception PDU must be 2 bytes");
    assert!(pdu[0] & 0x80 != 0, "exception FC must have 0x80 flag");
    (pdu[0], pdu[1])
}

// ── Unknown function code → IllegalFunction (0x01) ─────────────────

#[tokio::test]
async fn unknown_fc_returns_illegal_function() {
    let s = store();
    // FC 0x50 is not a public function code
    let pdu = [0x50, 0x00, 0x00, 0x00, 0x01];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    let (fc, ec) = parse_exception(&resp);
    assert_eq!(fc, 0x50 | 0x80, "exception FC should be 0xD0");
    assert_eq!(ec, ExceptionCode::IllegalFunction.code(), "unknown FC → IllegalFunction (0x01)");
}

// ── Known FC with truncated data → IllegalDataValue (0x03) ─────────

#[tokio::test]
async fn fc01_truncated_returns_illegal_data_value() {
    let s = store();
    // FC 01 (Read Coils) needs 4 data bytes, only sending 2
    let pdu = [0x01, 0x00, 0x13];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    let (fc, ec) = parse_exception(&resp);
    assert_eq!(fc, 0x81);
    assert_eq!(ec, ExceptionCode::IllegalDataValue.code(),
        "known FC with truncated data → IllegalDataValue (0x03), not IllegalFunction (0x01)");
}

#[tokio::test]
async fn fc03_truncated_returns_illegal_data_value() {
    let s = store();
    // FC 03 needs 4 data bytes, only sending 1
    let pdu = [0x03, 0x00];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    let (_, ec) = parse_exception(&resp);
    assert_eq!(ec, ExceptionCode::IllegalDataValue.code());
}

// ── Known FC with quantity out of range → IllegalDataValue (0x03) ──

#[tokio::test]
async fn fc01_quantity_zero_returns_illegal_data_value() {
    let s = store();
    // FC 01 with quantity = 0 (invalid per §6.1)
    let pdu = [0x01, 0x00, 0x00, 0x00, 0x00];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    let (fc, ec) = parse_exception(&resp);
    assert_eq!(fc, 0x81);
    assert_eq!(ec, ExceptionCode::IllegalDataValue.code(),
        "FC01 quantity=0 → IllegalDataValue (0x03)");
}

#[tokio::test]
async fn fc01_quantity_above_max_returns_illegal_data_value() {
    let s = store();
    // FC 01 with quantity = 2001 (max is 2000)
    let pdu = [0x01, 0x00, 0x00, 0x07, 0xD1];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    let (_, ec) = parse_exception(&resp);
    assert_eq!(ec, ExceptionCode::IllegalDataValue.code(),
        "FC01 quantity=2001 → IllegalDataValue (0x03)");
}

#[tokio::test]
async fn fc03_quantity_above_max_returns_illegal_data_value() {
    let s = store();
    // FC 03 with quantity = 126 (max is 125)
    let pdu = [0x03, 0x00, 0x00, 0x00, 0x7E];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    let (_, ec) = parse_exception(&resp);
    assert_eq!(ec, ExceptionCode::IllegalDataValue.code(),
        "FC03 quantity=126 → IllegalDataValue (0x03)");
}

// ── FC 05 invalid coil value → IllegalDataValue (0x03) ─────────────

#[tokio::test]
async fn fc05_invalid_coil_value_returns_illegal_data_value() {
    let s = store();
    // FC 05 with value 0x0001 (only 0x0000 and 0xFF00 are valid per §6.5)
    let pdu = [0x05, 0x00, 0x00, 0x00, 0x01];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    let (fc, ec) = parse_exception(&resp);
    assert_eq!(fc, 0x85);
    assert_eq!(ec, ExceptionCode::IllegalDataValue.code(),
        "FC05 invalid coil value → IllegalDataValue (0x03)");
}

// ── FC 0F byte count mismatch → IllegalDataValue (0x03) ────────────

#[tokio::test]
async fn fc0f_byte_count_wire_mismatch_returns_illegal_data_value() {
    let s = store();
    // FC 0F: quantity=8, byte_count=3 but only 2 data bytes follow → ByteCountMismatch
    let pdu = [0x0F, 0x00, 0x00, 0x00, 0x08, 0x03, 0xFF, 0x00];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    let (_, ec) = parse_exception(&resp);
    assert_eq!(ec, ExceptionCode::IllegalDataValue.code(),
        "FC0F byte count wire mismatch → IllegalDataValue (0x03)");
}

// ── Valid requests return normal responses ──────────────────────────

#[tokio::test]
async fn fc01_valid_request_succeeds() {
    let s = store();
    // FC 01: read 8 coils at address 0
    let pdu = [0x01, 0x00, 0x00, 0x00, 0x08];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    assert_eq!(resp[0], 0x01, "normal response echoes FC");
    assert_eq!(resp[1], 1, "byte_count = ceil(8/8) = 1");
}

#[tokio::test]
async fn fc03_valid_request_succeeds() {
    let s = store();
    // FC 03: read 3 registers at address 0
    let pdu = [0x03, 0x00, 0x00, 0x00, 0x03];
    let resp = handler::process_request(&pdu, UnitId(1), s.as_ref(), &DeviceIdentification::default()).await.unwrap();
    assert_eq!(resp[0], 0x03, "normal response echoes FC");
    assert_eq!(resp[1], 6, "byte_count = 3 * 2 = 6");
}

// ── Broadcast suppresses response ──────────────────────────────────

#[tokio::test]
async fn broadcast_read_suppressed() {
    let s = store();
    let pdu = [0x01, 0x00, 0x00, 0x00, 0x08];
    let resp = handler::process_request(&pdu, UnitId(0), s.as_ref(), &DeviceIdentification::default()).await;
    assert!(resp.is_none(), "broadcast read should return None");
}

#[tokio::test]
async fn broadcast_write_suppressed() {
    let s = store();
    // FC 06: write single register
    let pdu = [0x06, 0x00, 0x00, 0x00, 0x42];
    let resp = handler::process_request(&pdu, UnitId(0), s.as_ref(), &DeviceIdentification::default()).await;
    assert!(resp.is_none(), "broadcast write should return None");
}
