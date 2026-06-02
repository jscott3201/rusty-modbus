//! Integration tests for `ModbusServer` — uses `ModbusClient` to exercise the full stack.

use std::sync::Arc;
use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_server::{
    DataStore, InMemoryStore, ModbusServer, ServerConfig, StoreConfig, StoreError,
};
use rusty_modbus_types::{ExceptionCode, UnitId};

async fn start_server_with_store(
    store: Arc<InMemoryStore>,
) -> (ModbusServer<InMemoryStore>, std::net::SocketAddr) {
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

#[tokio::test]
async fn read_holding_registers() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(0, 0x1234).unwrap();
    store.set_holding_register(1, 0x5678).unwrap();

    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let regs = client
        .read_holding_registers(UnitId(1), 0, 2)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x1234, 0x5678]);
}

#[tokio::test]
async fn read_input_registers() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_input_register(10, 0xAAAA).unwrap();

    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let regs = client.read_input_registers(UnitId(1), 10, 1).await.unwrap();
    assert_eq!(regs, vec![0xAAAA]);
}

#[tokio::test]
async fn write_and_read_back_register() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    client
        .write_single_register(UnitId(1), 5, 0xBEEF)
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 5, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0xBEEF]);
}

#[tokio::test]
async fn write_multiple_and_read_back() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    client
        .write_multiple_registers(UnitId(1), 0, &[0x0001, 0x0002, 0x0003])
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 3)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x0001, 0x0002, 0x0003]);
}

#[tokio::test]
async fn direct_bulk_register_write_over_store_capacity_returns_error() {
    let store = InMemoryStore::new(StoreConfig::default());
    let values = vec![0x1234; 65_537];

    let result = store.write_registers(0, &values).await;

    assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
}

#[tokio::test]
async fn direct_packed_register_write_reads_back() {
    let store = InMemoryStore::new(StoreConfig::default());
    let values = [0x12, 0x34, 0xAB, 0xCD, 0x00, 0x01];

    store.write_registers_be(10, 3, &values).await.unwrap();

    let mut read = [0u16; 3];
    let count = store
        .read_holding_registers(10, 3, &mut read)
        .await
        .unwrap();
    assert_eq!(count, 3);
    assert_eq!(read, [0x1234, 0xABCD, 0x0001]);
}

#[tokio::test]
async fn direct_packed_register_write_bad_byte_count_is_illegal_data_value() {
    let store = InMemoryStore::new(StoreConfig::default());

    let result = store.write_registers_be(0, 2, &[0x12, 0x34]).await;

    assert_eq!(result, Err(ExceptionCode::IllegalDataValue));
}

#[tokio::test]
async fn direct_packed_register_write_over_store_capacity_returns_error() {
    let store = InMemoryStore::new(StoreConfig::default());
    let values = [0x12, 0x34, 0x56, 0x78];

    let result = store.write_registers_be(u16::MAX, 2, &values).await;

    assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
}

#[tokio::test]
async fn read_coils() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_coil(0, true).unwrap();
    store.set_coil(1, false).unwrap();
    store.set_coil(2, true).unwrap();

    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let coils = client.read_coils(UnitId(1), 0, 3).await.unwrap();
    assert_eq!(coils, vec![true, false, true]);
}

#[tokio::test]
async fn write_single_coil_and_read_back() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    client.write_single_coil(UnitId(1), 7, true).await.unwrap();
    let coils = client.read_coils(UnitId(1), 7, 1).await.unwrap();
    assert_eq!(coils, vec![true]);
}

#[tokio::test]
async fn direct_bulk_coil_write_over_store_capacity_returns_error() {
    let store = InMemoryStore::new(StoreConfig::default());
    let values = vec![true; 65_537];

    let result = store.write_coils(0, &values).await;

    assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
}

#[tokio::test]
async fn direct_packed_coil_write_reads_back() {
    let store = InMemoryStore::new(StoreConfig::default());

    store
        .write_coils_packed(20, 9, &[0b1000_0101, 0b0000_0001])
        .await
        .unwrap();

    let mut read = [false; 9];
    let count = store.read_coils(20, 9, &mut read).await.unwrap();
    assert_eq!(count, 9);
    assert_eq!(
        read,
        [true, false, true, false, false, false, false, true, true]
    );
}

#[tokio::test]
async fn direct_packed_coil_read_writes_wire_bytes() {
    let store = InMemoryStore::new(StoreConfig::default());
    for address in [20, 22, 27, 28] {
        store.set_coil(address, true).unwrap();
    }

    let mut packed = [0xFF, 0xFF];
    let count = store.read_coils_packed(20, 9, &mut packed).await.unwrap();

    assert_eq!(count, 9);
    assert_eq!(packed, [0b1000_0101, 0b0000_0001]);
}

#[tokio::test]
async fn direct_packed_coil_read_bad_output_len_is_illegal_data_value() {
    let store = InMemoryStore::new(StoreConfig::default());
    let mut packed = [0u8; 1];

    let result = store.read_coils_packed(0, 9, &mut packed).await;

    assert_eq!(result, Err(ExceptionCode::IllegalDataValue));
}

#[tokio::test]
async fn direct_packed_discrete_input_read_writes_wire_bytes() {
    let store = InMemoryStore::new(StoreConfig::default());
    for address in [3, 4, 10, 12] {
        store.set_discrete_input(address, true).unwrap();
    }

    let mut packed = [0xFF, 0xFF];
    let count = store
        .read_discrete_inputs_packed(3, 10, &mut packed)
        .await
        .unwrap();

    assert_eq!(count, 10);
    assert_eq!(packed, [0b1000_0011, 0b0000_0010]);
}

#[tokio::test]
async fn direct_packed_coil_write_bad_byte_count_is_illegal_data_value() {
    let store = InMemoryStore::new(StoreConfig::default());

    let result = store.write_coils_packed(0, 9, &[0b0000_0001]).await;

    assert_eq!(result, Err(ExceptionCode::IllegalDataValue));
}

#[tokio::test]
async fn direct_packed_coil_write_over_store_capacity_returns_error() {
    let store = InMemoryStore::new(StoreConfig::default());

    let result = store.write_coils_packed(u16::MAX, 2, &[0b0000_0011]).await;

    assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
}

#[tokio::test]
async fn concurrent_clients() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(0, 0x42).unwrap();

    let (_server, addr) = start_server_with_store(store).await;

    let mut handles = Vec::new();
    for _ in 0..3 {
        let a = addr;
        handles.push(tokio::spawn(async move {
            let client = ModbusClient::connect(
                a,
                ClientConfig {
                    timeout: Duration::from_secs(2),
                    ..ClientConfig::default()
                },
            )
            .await
            .unwrap();
            client.read_holding_registers(UnitId(1), 0, 1).await
        }));
    }

    for h in handles {
        let result = h.await.unwrap().unwrap();
        assert_eq!(result, vec![0x42]);
    }
}

#[tokio::test]
async fn mask_write_register() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(4, 0x00FF).unwrap();

    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // (0x00FF AND 0x00F2) OR (0x0025 AND NOT(0x00F2)) = 0x00F2 | 0x0005 = 0x00F7
    // Wait: (0x00FF & 0x00F2) | (0x0025 & !0x00F2)
    // = 0x00F2 | (0x0025 & 0xFF0D) = 0x00F2 | 0x0005 = 0x00F7
    client
        .mask_write_register(UnitId(1), 4, 0x00F2, 0x0025)
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 4, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x00F7]);
}

#[test]
fn oversized_store_config_is_rejected_before_allocation() {
    let config = StoreConfig {
        holding_register_count: 65_537,
        ..StoreConfig::default()
    };

    let err = InMemoryStore::try_new(config).unwrap_err();
    assert_eq!(
        err,
        StoreError::TableTooLarge {
            table: "holding_registers",
            count: 65_537,
            max: 65_536,
        }
    );
}

#[test]
fn setup_write_outside_configured_table_returns_error() {
    let store = InMemoryStore::new(StoreConfig {
        holding_register_count: 1,
        ..StoreConfig::default()
    });

    assert_eq!(
        store.set_holding_register(1, 0xBEEF),
        Err(StoreError::AddressOutOfRange {
            table: "holding_registers",
            address: 1,
            len: 1,
        })
    );
}

#[test]
fn file_setup_rejects_file_zero() {
    let store = InMemoryStore::new(StoreConfig::default());

    assert_eq!(
        store.set_file_record(0, 0, 0xBEEF),
        Err(StoreError::FileNumberOutOfRange {
            file_number: 0,
            minimum: 1,
        })
    );
}

#[test]
fn file_setup_rejects_record_outside_spec_range() {
    let store = InMemoryStore::new(StoreConfig::default());

    assert_eq!(
        store.set_file_record(1, 0x2710, 0xBEEF),
        Err(StoreError::FileRecordOutOfRange {
            record_number: 0x2710,
            maximum: 0x270F,
        })
    );
}

#[tokio::test]
async fn server_stop_rejects_new_connections() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let (server, addr) = start_server_with_store(store).await;

    // Verify it works.
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();
    client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    drop(client);

    // Stop server.
    server.stop().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // New connection should fail.
    let result = ModbusClient::connect(
        addr,
        ClientConfig {
            timeout: Duration::from_millis(500),
            ..ClientConfig::default()
        },
    )
    .await;
    // Server either refuses the connection or the client times out.
    assert!(
        result.is_err() || {
            // If connect succeeded, a request should fail.
            let c = result.unwrap();
            c.read_holding_registers(UnitId(1), 0, 1).await.is_err()
        }
    );
}
