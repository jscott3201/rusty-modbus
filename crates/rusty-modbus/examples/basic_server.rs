//! Basic Modbus/TCP server — in-memory store with pre-populated registers.
//!
//! Run: `cargo run -p rusty-modbus --features full --example basic_server`

use std::sync::Arc;

use rusty_modbus::server::{ModbusServer, ServerConfig, InMemoryStore, StoreConfig};
use rusty_modbus::types::UnitId;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));

    for i in 0u16..10 {
        store.set_holding_register(i, i * 100);
    }

    let config = ServerConfig {
        listen_addr: "0.0.0.0:5020".parse()?,
        unit_id: UnitId(1),
        ..ServerConfig::default()
    };

    let server = ModbusServer::start(config, store).await?;
    println!("Modbus server on {}", server.local_addr());
    println!("Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    server.stop().await;
    Ok(())
}
