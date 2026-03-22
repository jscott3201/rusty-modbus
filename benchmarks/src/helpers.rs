//! Shared helpers for benchmark setup.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use modbus_client::config::{ClientConfig, RetryConfig};
use modbus_client::ModbusClient;
use modbus_server::config::ServerConfig;
use modbus_server::store::memory::{InMemoryStore, StoreConfig};
use modbus_server::ModbusServer;
use modbus_types::UnitId;

/// Create a pre-populated in-memory store for benchmarks.
pub fn make_store() -> Arc<InMemoryStore> {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    populate_store(&store);
    store
}

/// Fill store with test data: holding registers 0..99 and coils 0..999.
pub fn populate_store(store: &InMemoryStore) {
    for i in 0u16..100 {
        store.set_holding_register(i, 1000 + i);
    }
    for i in 0u16..1000 {
        store.set_coil(i, i % 2 == 0);
    }
}

/// Start a TCP server on an ephemeral port with a pre-populated store.
pub async fn make_tcp_server() -> (ModbusServer<InMemoryStore>, SocketAddr) {
    let store = make_store();
    make_tcp_server_with_store(store).await
}

/// Start a TCP server on an ephemeral port with the given store.
pub async fn make_tcp_server_with_store(
    store: Arc<InMemoryStore>,
) -> (ModbusServer<InMemoryStore>, SocketAddr) {
    let config = ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        ..ServerConfig::default()
    };
    let server = ModbusServer::start(config, store).await.unwrap();
    let addr = server.local_addr();
    (server, addr)
}

/// Connect a `ModbusClient` to the given address with benchmark-tuned config.
pub async fn make_tcp_client(addr: SocketAddr) -> ModbusClient {
    let config = ClientConfig {
        unit_id: UnitId(1),
        timeout: Duration::from_secs(5),
        retry: RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };
    ModbusClient::connect(addr, config).await.unwrap()
}

/// Get current process RSS in bytes.
pub fn current_rss_bytes() -> u64 {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from(std::process::id() as usize);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}
