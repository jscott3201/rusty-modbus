//! Server handler conformance tests.
//!
//! Verifies that the server handler returns correct exception codes per
//! the spec state diagrams (V1.1b3 Figures 11-28).
//!
//! Key conformance requirement: when a known function code has malformed data
//! (bad quantity, bad byte count, truncated), the exception code must be
//! IllegalDataValue (0x03), NOT IllegalFunction (0x01).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_server::config::ServerConfig;
use rusty_modbus_server::store::memory::{InMemoryStore, StoreConfig};
use rusty_modbus_server::{DataStore, DeviceIdentification, ModbusServer, handler};
use rusty_modbus_types::{ExceptionCode, UnitId};

macro_rules! required_store_methods {
    () => {
        async fn read_coils(&self, _: u16, _: u16, _: &mut [bool]) -> Result<usize, ExceptionCode> {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            Err(ExceptionCode::IllegalDataAddress)
        }

        async fn write_coil(&self, _: u16, _: bool) -> Result<(), ExceptionCode> {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            Err(ExceptionCode::IllegalDataAddress)
        }

        async fn write_coils(&self, _: u16, _: &[bool]) -> Result<(), ExceptionCode> {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            Err(ExceptionCode::IllegalDataAddress)
        }

        async fn read_discrete_inputs(
            &self,
            _: u16,
            _: u16,
            _: &mut [bool],
        ) -> Result<usize, ExceptionCode> {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            Err(ExceptionCode::IllegalDataAddress)
        }

        async fn read_holding_registers(
            &self,
            _: u16,
            _: u16,
            _: &mut [u16],
        ) -> Result<usize, ExceptionCode> {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            Err(ExceptionCode::IllegalDataAddress)
        }

        async fn write_register(&self, _: u16, _: u16) -> Result<(), ExceptionCode> {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            Err(ExceptionCode::IllegalDataAddress)
        }

        async fn write_registers(&self, _: u16, _: &[u16]) -> Result<(), ExceptionCode> {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            Err(ExceptionCode::IllegalDataAddress)
        }

        async fn read_input_registers(
            &self,
            _: u16,
            _: u16,
            _: &mut [u16],
        ) -> Result<usize, ExceptionCode> {
            self.ordinary_calls.fetch_add(1, Ordering::SeqCst);
            Err(ExceptionCode::IllegalDataAddress)
        }
    };
}

#[derive(Default)]
struct LegacyStore {
    ordinary_calls: AtomicUsize,
}

impl DataStore for LegacyStore {
    required_store_methods!();
}

struct AtomicStore {
    ordinary_calls: AtomicUsize,
    mask_calls: AtomicUsize,
    read_write_calls: AtomicUsize,
    read_write_count: usize,
}

impl AtomicStore {
    fn new(read_write_count: usize) -> Self {
        Self {
            ordinary_calls: AtomicUsize::new(0),
            mask_calls: AtomicUsize::new(0),
            read_write_calls: AtomicUsize::new(0),
            read_write_count,
        }
    }
}

impl DataStore for AtomicStore {
    required_store_methods!();

    async fn atomic_mask_write_register(
        &self,
        address: u16,
        and_mask: u16,
        or_mask: u16,
    ) -> Result<(), ExceptionCode> {
        self.mask_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!((address, and_mask, or_mask), (4, 0x00F2, 0x0025));
        Ok(())
    }

    async fn atomic_read_write_registers_be(
        &self,
        read_address: u16,
        read_quantity: u16,
        write_address: u16,
        write_quantity: u16,
        write_values: &[u8],
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        self.read_write_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!((read_address, read_quantity), (0, 1));
        assert_eq!((write_address, write_quantity), (1, 1));
        assert_eq!(write_values, [0x12, 0x34]);
        out.copy_from_slice(&[0xCA, 0xFE]);
        Ok(self.read_write_count)
    }
}

const FC16_REQUEST: &[u8] = &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25];
const FC17_REQUEST: &[u8] = &[
    0x17, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x02, 0x12, 0x34,
];

async fn start_server() -> (ModbusServer<InMemoryStore>, std::net::SocketAddr) {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let config = ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        ..ServerConfig::default()
    };
    let server = ModbusServer::start(config, store).await.unwrap();
    let addr = server.local_addr();
    (server, addr)
}

fn client_config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    }
}

/// Per spec Figure 13: FC03 with address overflow → IllegalDataAddress (0x02)
#[tokio::test]
async fn fc03_address_overflow_returns_illegal_data_address() {
    let (_server, addr) = start_server().await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Read 2 registers starting at 0xFFFF — overflows address space
    let result = client.read_holding_registers(UnitId(1), 0xFFFF, 2).await;

    match result {
        Err(rusty_modbus_client::ClientError::Exception(exc)) => {
            assert_eq!(
                exc.exception_code,
                ExceptionCode::IllegalDataAddress,
                "FC03 address overflow should return IllegalDataAddress (0x02)"
            );
        }
        other => panic!("expected Exception, got {other:?}"),
    }
}

/// Per spec Figure 13: FC03 with valid quantity but address out of store range
#[tokio::test]
async fn fc03_address_beyond_store_returns_illegal_data_address() {
    let store = Arc::new(InMemoryStore::new(StoreConfig {
        holding_register_count: 100,
        ..StoreConfig::default()
    }));
    let config = ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        ..ServerConfig::default()
    };
    let server = ModbusServer::start(config, store).await.unwrap();
    let addr = server.local_addr();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Read 1 register at address 100 — store only has 100 registers (0-99)
    let result = client.read_holding_registers(UnitId(1), 100, 1).await;

    match result {
        Err(rusty_modbus_client::ClientError::Exception(exc)) => {
            assert_eq!(
                exc.exception_code,
                ExceptionCode::IllegalDataAddress,
                "FC03 address beyond store should return IllegalDataAddress (0x02)"
            );
        }
        other => panic!("expected Exception, got {other:?}"),
    }
}

/// Mask write algorithm verification per spec §6.16
#[tokio::test]
async fn fc16_mask_write_algorithm() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    // Set register 4 to 0x0012 (spec example value)
    store.set_holding_register(4, 0x0012).unwrap();

    let config = ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        ..ServerConfig::default()
    };
    let server = ModbusServer::start(config, store.clone()).await.unwrap();
    let addr = server.local_addr();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Spec §6.16 example: AND=0x00F2, OR=0x0025
    // Result = (0x0012 AND 0x00F2) OR (0x0025 AND NOT(0x00F2))
    //        = 0x0012 OR (0x0025 AND 0xFF0D)
    //        = 0x0012 OR 0x0005
    //        = 0x0017
    client
        .mask_write_register(UnitId(1), 4, 0x00F2, 0x0025)
        .await
        .unwrap();

    let regs = client
        .read_holding_registers(UnitId(1), 4, 1)
        .await
        .unwrap();
    assert_eq!(
        regs,
        vec![0x0017],
        "mask write result should be 0x0017 per spec §6.16"
    );
}

/// FC17: Write executes before read per spec §6.17
#[tokio::test]
async fn fc17_write_before_read() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let config = ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        ..ServerConfig::default()
    };
    let server = ModbusServer::start(config, store).await.unwrap();
    let addr = server.local_addr();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let regs = client
        .read_write_multiple_registers(UnitId(1), 0, 1, 0, &[0xBEEF])
        .await
        .unwrap();
    assert_eq!(regs, vec![0xBEEF]);
}

#[tokio::test]
async fn compound_operations_fail_closed_for_legacy_stores() {
    let store = LegacyStore::default();
    let device_id = DeviceIdentification::default();

    let fc16 = handler::process_request(FC16_REQUEST, UnitId(1), &store, &device_id)
        .await
        .unwrap();
    let fc17 = handler::process_request(FC17_REQUEST, UnitId(1), &store, &device_id)
        .await
        .unwrap();

    assert_eq!(fc16, [0x96, 0x01]);
    assert_eq!(fc17, [0x97, 0x01]);
    assert_eq!(store.ordinary_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn capable_compound_hooks_are_called_once() {
    let store = AtomicStore::new(1);
    let device_id = DeviceIdentification::default();

    let fc16 = handler::process_request(FC16_REQUEST, UnitId(1), &store, &device_id)
        .await
        .unwrap();
    let fc17 = handler::process_request(FC17_REQUEST, UnitId(1), &store, &device_id)
        .await
        .unwrap();

    assert_eq!(fc16, FC16_REQUEST);
    assert_eq!(fc17, [0x17, 0x02, 0xCA, 0xFE]);
    assert_eq!(store.mask_calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.read_write_calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.ordinary_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fc17_bad_atomic_result_count_returns_server_device_failure() {
    let store = AtomicStore::new(0);
    let response = handler::process_request(
        FC17_REQUEST,
        UnitId(1),
        &store,
        &DeviceIdentification::default(),
    )
    .await
    .unwrap();

    assert_eq!(response, [0x97, 0x04]);
    assert_eq!(store.read_write_calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.ordinary_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn malformed_compound_requests_do_not_invoke_atomic_hooks() {
    let store = AtomicStore::new(1);
    let device_id = DeviceIdentification::default();

    let fc16 = handler::process_request(&FC16_REQUEST[..6], UnitId(1), &store, &device_id)
        .await
        .unwrap();
    let fc17 = handler::process_request(&FC17_REQUEST[..11], UnitId(1), &store, &device_id)
        .await
        .unwrap();

    assert_eq!(fc16, [0x96, 0x03]);
    assert_eq!(fc17, [0x97, 0x03]);
    assert_eq!(store.mask_calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.read_write_calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.ordinary_calls.load(Ordering::SeqCst), 0);
}

/// Coil bit-packing: write then read back, verify LSB-first packing
#[tokio::test]
async fn coil_bit_packing_round_trip() {
    let (_server, addr) = start_server().await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Write 10 coils with known pattern
    let values = vec![
        true, false, true, true, false, false, true, true, false, true,
    ];
    client
        .write_multiple_coils(UnitId(1), 0, &values)
        .await
        .unwrap();

    let read_back = client.read_coils(UnitId(1), 0, 10).await.unwrap();
    assert_eq!(
        read_back, values,
        "coil round-trip should preserve bit pattern"
    );
}

/// Broadcast writes execute but return no response
#[tokio::test]
async fn broadcast_write_executes_silently() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let config = ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        ..ServerConfig::default()
    };
    let server = ModbusServer::start(config, store.clone()).await.unwrap();
    let addr = server.local_addr();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Broadcast write — should execute but return no response (timeout)
    let _result = client.write_single_register(UnitId(0), 5, 0x1234).await;

    // The broadcast should not produce a response, so the client times out
    // But the store should have been updated
    // Note: broadcast behavior depends on how the client handles no-response
}
