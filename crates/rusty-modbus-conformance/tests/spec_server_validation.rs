//! Spec V1.1b3 §4.5 — the **server's real request path** returns spec-correct
//! exception codes, in the right order.
//!
//! `spec_validation.rs` exercises the standalone `rusty_modbus_codec::validate`
//! helpers. Those helpers are a public utility — but the server does NOT call
//! them: it validates quantity while decoding (`decode_request` →
//! `IllegalDataValue`) and validates address in the data store
//! (`IllegalDataAddress`). This file drives `handler::process_request`
//! end-to-end (decode → store) so the behavior callers actually observe is
//! verified, not just the unused validator.
//!
//! Per the spec state diagrams (Figures 11–28), the data-value check
//! (`IllegalDataValue`, 0x03) MUST happen before the address check
//! (`IllegalDataAddress`, 0x02): when both fail, the response is 0x03.

use rusty_modbus_server::handler::process_request;
use rusty_modbus_server::{DeviceIdentification, InMemoryStore, StoreConfig};
use rusty_modbus_types::UnitId;

const UNIT: UnitId = UnitId(1);

/// A small store so address-overflow is unambiguous (100-entry tables).
fn small_store() -> InMemoryStore {
    InMemoryStore::new(StoreConfig {
        coil_count: 100,
        discrete_input_count: 100,
        holding_register_count: 100,
        input_register_count: 100,
    })
}

/// Drive the real server path and return the response PDU (panics on broadcast).
async fn respond(pdu: &[u8]) -> Vec<u8> {
    let store = small_store();
    process_request(pdu, UNIT, &store, &DeviceIdentification::default())
        .await
        .expect("a non-broadcast request must produce a response")
}

/// Build a read request PDU: function code, start address, quantity.
fn read_pdu(fc: u8, address: u16, quantity: u16) -> Vec<u8> {
    let mut v = vec![fc];
    v.extend_from_slice(&address.to_be_bytes());
    v.extend_from_slice(&quantity.to_be_bytes());
    v
}

// ── Quantity (data value) → IllegalDataValue (0x03) ───────────────

#[tokio::test]
async fn fc03_over_quantity_is_illegal_data_value() {
    // 200 > 125 (max holding registers)
    assert_eq!(respond(&read_pdu(0x03, 0, 200)).await, vec![0x83, 0x03]);
}

#[tokio::test]
async fn fc01_over_quantity_is_illegal_data_value() {
    // 3000 > 2000 (max coils)
    assert_eq!(respond(&read_pdu(0x01, 0, 3000)).await, vec![0x81, 0x03]);
}

#[tokio::test]
async fn fc05_invalid_coil_value_is_illegal_data_value() {
    // Write Single Coil value must be 0x0000 or 0xFF00; 0x1234 is invalid.
    let pdu = vec![0x05, 0x00, 0x00, 0x12, 0x34];
    assert_eq!(respond(&pdu).await, vec![0x85, 0x03]);
}

// ── Address → IllegalDataAddress (0x02) ───────────────────────────

#[tokio::test]
async fn fc03_address_out_of_range_is_illegal_data_address() {
    // quantity 20 is valid, but 90 + 20 = 110 > 100 → address error.
    assert_eq!(respond(&read_pdu(0x03, 90, 20)).await, vec![0x83, 0x02]);
}

// ── Ordering: data value (0x03) wins over address (0x02) ──────────

#[tokio::test]
async fn fc03_quantity_checked_before_address() {
    // Both fail: quantity 200 > 125 AND 0xFFFF + 200 overflows the table.
    // The spec requires the data-value check first → 0x03, never 0x02.
    assert_eq!(
        respond(&read_pdu(0x03, 0xFFFF, 200)).await,
        vec![0x83, 0x03],
        "IllegalDataValue (0x03) must be returned before IllegalDataAddress (0x02)"
    );
}

#[tokio::test]
async fn fc01_quantity_checked_before_address() {
    assert_eq!(
        respond(&read_pdu(0x01, 0xFFFF, 3000)).await,
        vec![0x81, 0x03]
    );
}

// ── Unknown function code → IllegalFunction (0x01) ────────────────

#[tokio::test]
async fn unknown_function_code_is_illegal_function() {
    // 0x65 is unassigned → IllegalFunction with the exception flag set.
    assert_eq!(respond(&[0x65, 0x00, 0x00]).await, vec![0x65 | 0x80, 0x01]);
}

#[tokio::test]
async fn zero_function_code_is_illegal_function() {
    // Function code 0 is invalid per Modbus Application Protocol §4.1.
    assert_eq!(respond(&[0x00]).await, vec![0x80, 0x01]);
}

// ── Happy path still works through the real dispatch ──────────────

#[tokio::test]
async fn valid_read_returns_normal_response() {
    let resp = respond(&read_pdu(0x03, 0, 2)).await;
    assert_eq!(resp[0], 0x03, "echoes the function code (not an exception)");
    assert_eq!(resp[1], 4, "byte count = 2 registers * 2 bytes");
    assert_eq!(resp.len(), 2 + 4);
}

// ── Broadcast writes produce no response ──────────────────────────

#[tokio::test]
async fn broadcast_write_produces_no_response() {
    let store = small_store();
    let pdu = vec![0x06, 0x00, 0x00, 0x12, 0x34]; // Write Single Register
    let resp = process_request(&pdu, UnitId(0), &store, &DeviceIdentification::default()).await;
    assert!(
        resp.is_none(),
        "a broadcast request must not produce a response"
    );
}
