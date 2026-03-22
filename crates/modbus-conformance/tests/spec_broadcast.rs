//! Broadcast conformance tests.
//!
//! V1.1b3 §4: "Address 0 is used as broadcast address."
//! Broadcast writes execute on the server but produce no response.
//! Broadcast reads are undefined and should be rejected.

use std::sync::Arc;
use modbus_server::handler;
use modbus_server::store::DataStore;
use modbus_server::store::memory::{InMemoryStore, StoreConfig};
use modbus_server::DeviceIdentification;
use modbus_types::UnitId;

fn store() -> Arc<InMemoryStore> {
    Arc::new(InMemoryStore::new(StoreConfig::default()))
}

#[tokio::test]
async fn broadcast_write_single_register_executes() {
    let s = store();
    // FC 06: write register 5 = 0x1234, unit_id=0 (broadcast)
    let pdu = [0x06, 0x00, 0x05, 0x12, 0x34];
    let resp = handler::process_request(&pdu, UnitId(0), s.as_ref(), &DeviceIdentification::default()).await;

    // No response for broadcast
    assert!(resp.is_none(), "broadcast write should return None (no response)");

    // But the write should have executed
    let mut buf = [0u16; 1];
    let count = s.read_holding_registers(5, 1, &mut buf).await.unwrap();
    assert_eq!(count, 1);
    assert_eq!(buf[0], 0x1234, "broadcast write should have been executed");
}

#[tokio::test]
async fn broadcast_write_single_coil_executes() {
    let s = store();
    // FC 05: write coil 10 = ON (0xFF00), unit_id=0
    let pdu = [0x05, 0x00, 0x0A, 0xFF, 0x00];
    let resp = handler::process_request(&pdu, UnitId(0), s.as_ref(), &DeviceIdentification::default()).await;

    assert!(resp.is_none());

    let mut buf = [false; 1];
    let count = s.read_coils(10, 1, &mut buf).await.unwrap();
    assert_eq!(count, 1);
    assert!(buf[0], "broadcast coil write should have been executed");
}

#[tokio::test]
async fn broadcast_write_multiple_registers_executes() {
    let s = store();
    // FC 10: write 2 registers at address 0 = [0xAAAA, 0xBBBB], unit_id=0
    let pdu = [0x10, 0x00, 0x00, 0x00, 0x02, 0x04, 0xAA, 0xAA, 0xBB, 0xBB];
    let resp = handler::process_request(&pdu, UnitId(0), s.as_ref(), &DeviceIdentification::default()).await;

    assert!(resp.is_none());

    let mut buf = [0u16; 2];
    s.read_holding_registers(0, 2, &mut buf).await.unwrap();
    assert_eq!(buf, [0xAAAA, 0xBBBB]);
}

#[tokio::test]
async fn broadcast_read_returns_none() {
    let s = store();
    // FC 03: read holding registers, unit_id=0 (broadcast)
    // Per spec §4: reads to broadcast address produce no response
    let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
    let resp = handler::process_request(&pdu, UnitId(0), s.as_ref(), &DeviceIdentification::default()).await;
    assert!(resp.is_none(), "broadcast read should return None");
}

#[tokio::test]
async fn broadcast_unknown_fc_returns_none() {
    let s = store();
    // Unknown FC with broadcast — no response either
    let pdu = [0x50, 0x00, 0x00];
    let resp = handler::process_request(&pdu, UnitId(0), s.as_ref(), &DeviceIdentification::default()).await;
    assert!(resp.is_none(), "broadcast unknown FC should return None");
}
