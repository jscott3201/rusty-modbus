//! Integration tests for `ModbusServer` — uses `ModbusClient` to exercise the full stack.

use std::sync::Arc;
use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_server::{DataStore, InMemoryStore, ModbusServer, ServerConfig, StoreConfig};
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
    store.set_holding_register(0, 0x1234);
    store.set_holding_register(1, 0x5678);

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
    store.set_input_register(10, 0xAAAA);

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
async fn read_coils() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_coil(0, true);
    store.set_coil(1, false);
    store.set_coil(2, true);

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
async fn concurrent_clients() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(0, 0x42);

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
    store.set_holding_register(4, 0x00FF);

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
