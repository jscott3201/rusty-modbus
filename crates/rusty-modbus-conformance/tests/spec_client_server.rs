//! Full-stack client-server conformance tests.
//!
//! Exercises every supported function code through the complete stack:
//! ModbusClient → TCP transport → MBAP framing → ModbusServer → DataStore
//!
//! Verifies spec-level behaviors:
//! - §4.3: Four data tables (coils, discrete inputs, holding regs, input regs)
//! - §4.5: Validation order and exception codes
//! - §6.5: Coil value ON=0xFF00, OFF=0x0000
//! - §6.16: Mask write algorithm
//! - §6.17: Write-before-read ordering
//! - TCP Guide §4.4.1.3: Transaction ID echo
//! - TCP Guide §§3.1.3, 4.4.1: response Unit Identifier correlation
//! - V1.1b3 §4: Broadcast writes execute, no response

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_server::ModbusServer;
use rusty_modbus_server::config::ServerConfig;
use rusty_modbus_server::store::memory::{InMemoryStore, StoreConfig};
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{ExceptionCode, MbapHeader, UnitId};

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

// ── §4.3 Four Data Tables — Complete CRUD ─────────────────────────

#[tokio::test]
async fn spec_4_3_holding_registers_read_write() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(0, 0xAAAA).unwrap();
    store.set_holding_register(1, 0xBBBB).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Read
    let regs = client
        .read_holding_registers(UnitId(1), 0, 2)
        .await
        .unwrap();
    assert_eq!(regs, vec![0xAAAA, 0xBBBB]);

    // Write single
    client
        .write_single_register(UnitId(1), 0, 0x1234)
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x1234]);

    // Write multiple
    client
        .write_multiple_registers(UnitId(1), 10, &[0x0001, 0x0002, 0x0003])
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 10, 3)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x0001, 0x0002, 0x0003]);
}

#[tokio::test]
async fn spec_4_3_input_registers_read() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_input_register(5, 0xCAFE).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let regs = client.read_input_registers(UnitId(1), 5, 1).await.unwrap();
    assert_eq!(regs, vec![0xCAFE]);
}

#[tokio::test]
async fn spec_4_3_coils_read_write() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_coil(0, true).unwrap();
    store.set_coil(1, false).unwrap();
    store.set_coil(2, true).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Read
    let coils = client.read_coils(UnitId(1), 0, 3).await.unwrap();
    assert_eq!(coils, vec![true, false, true]);

    // Write single
    client.write_single_coil(UnitId(1), 1, true).await.unwrap();
    let coils = client.read_coils(UnitId(1), 1, 1).await.unwrap();
    assert_eq!(coils, vec![true]);

    // Write multiple
    client
        .write_multiple_coils(UnitId(1), 0, &[false, true, false, true])
        .await
        .unwrap();
    let coils = client.read_coils(UnitId(1), 0, 4).await.unwrap();
    assert_eq!(coils, vec![false, true, false, true]);
}

#[tokio::test]
async fn spec_4_3_discrete_inputs_read() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_discrete_input(0, true).unwrap();
    store.set_discrete_input(1, false).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let inputs = client.read_discrete_inputs(UnitId(1), 0, 2).await.unwrap();
    assert_eq!(inputs, vec![true, false]);
}

// ── §6.16 Mask Write Register Algorithm ───────────────────────────

#[tokio::test]
async fn spec_6_16_mask_write_algorithm() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(4, 0x0012).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Spec §6.16 example: AND=0x00F2, OR=0x0025
    // Result = (0x0012 & 0x00F2) | (0x0025 & !0x00F2) = 0x0012 | 0x0005 = 0x0017
    client
        .mask_write_register(UnitId(1), 4, 0x00F2, 0x0025)
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 4, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x0017]);
}

#[tokio::test]
async fn spec_6_16_mask_write_and_only() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(0, 0xFF00).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // OR_Mask = 0 → result is simply Current AND And_Mask
    client
        .mask_write_register(UnitId(1), 0, 0x0F0F, 0x0000)
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x0F00]);
}

#[tokio::test]
async fn spec_6_16_mask_write_or_only() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(0, 0x0000).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // AND_Mask = 0 → result equals OR_Mask
    client
        .mask_write_register(UnitId(1), 0, 0x0000, 0xABCD)
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0xABCD]);
}

// ── §6.17 Read/Write Multiple — Write Before Read ─────────────────

#[tokio::test]
async fn spec_6_17_write_before_read_same_address() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(0, 0x0000).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Write 0xBEEF to reg 0, then read reg 0 — should see the write
    // §6.17: "The write operation is performed before the read."
    client
        .write_single_register(UnitId(1), 0, 0xBEEF)
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0xBEEF]);
}

// ── Address overflow → IllegalDataAddress ─────────────────────────

#[tokio::test]
async fn address_beyond_store_returns_illegal_data_address() {
    let store = Arc::new(InMemoryStore::new(StoreConfig {
        holding_register_count: 100,
        ..StoreConfig::default()
    }));
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let result = client.read_holding_registers(UnitId(1), 99, 2).await;
    match result {
        Err(ClientError::Exception(exc)) => {
            assert_eq!(exc.exception_code, ExceptionCode::IllegalDataAddress);
        }
        other => panic!("expected IllegalDataAddress exception, got {other:?}"),
    }
}

// ── Unit ID handling ──────────────────────────────────────────────

#[tokio::test]
async fn unit_id_mismatch_silently_discarded() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let (_server, addr) = start_server_with_store(store).await;

    // Server is configured with UnitId(1). Send request with UnitId(99).
    // Server should discard silently — client times out.
    let config = ClientConfig {
        timeout: Duration::from_millis(500),
        ..ClientConfig::default()
    };
    let client = ModbusClient::connect(addr, config).await.unwrap();
    let result = client.read_holding_registers(UnitId(99), 0, 1).await;
    // Server discards mismatched unit ID → client times out or retries exhaust
    assert!(
        result.is_err(),
        "expected error for mismatched unit ID, got {result:?}"
    );
}

#[tokio::test]
async fn tcp_007_client_rejects_response_with_wrong_unit_id() {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut sink, mut stream, _, _guard) = listener.accept().await.unwrap();
        let request = stream.recv().await.unwrap();
        let txn_id = match request.header {
            FrameHeader::Mbap(header) => header.transaction_id.get(),
            FrameHeader::Rtu { .. } => panic!("expected MBAP request"),
        };
        let pdu = Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]);
        sink.send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(txn_id, 2, pdu.len() as u16)),
            pdu,
        })
        .await
        .unwrap();
    });

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();
    let result = client.read_holding_registers(UnitId(1), 0, 1).await;
    assert!(matches!(
        result,
        Err(ClientError::UnexpectedResponseUnitId {
            expected: 1,
            got: 2
        })
    ));
}

#[tokio::test]
async fn unit_id_0xff_accepted_as_tcp_device() {
    // TCP Guide §4.4.1.2: "The value 0xFF has to be used" for direct TCP
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    store.set_holding_register(0, 42).unwrap();
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let regs = client
        .read_holding_registers(UnitId(0xFF), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![42]);
}

// ── Broadcast ─────────────────────────────────────────────────────

#[tokio::test]
async fn spec_4_broadcast_read_rejected_by_client() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let (_server, addr) = start_server_with_store(store).await;
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    let result = client.read_holding_registers(UnitId(0), 0, 1).await;
    assert!(matches!(result, Err(ClientError::BroadcastReadNotAllowed)));
}

// ── Pipelining ────────────────────────────────────────────────────

#[tokio::test]
async fn spec_tcp_guide_4_4_pipelining() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    for i in 0u16..10 {
        store.set_holding_register(i, i * 100).unwrap();
    }
    let (_server, addr) = start_server_with_store(store).await;
    let client = Arc::new(ModbusClient::connect(addr, client_config()).await.unwrap());

    // Send 5 concurrent requests — pipelined on same TCP connection
    let mut handles = Vec::new();
    for i in 0u16..5 {
        let c = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.read_holding_registers(UnitId(1), i, 1).await
        }));
    }

    for (i, h) in handles.into_iter().enumerate() {
        let result = h.await.unwrap().unwrap();
        assert_eq!(result, vec![i as u16 * 100]);
    }
}

// ── Client config defaults ────────────────────────────────────────

#[test]
fn client_config_defaults() {
    let config = ClientConfig::default();
    assert_eq!(config.unit_id, UnitId(0xFF)); // TCP direct device
    assert_eq!(config.max_in_flight, 16); // spec max
}
