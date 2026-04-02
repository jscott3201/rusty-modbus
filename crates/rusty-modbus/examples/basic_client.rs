//! Basic Modbus/TCP client — connect, read registers, print values.
//!
//! Run: `cargo run -p rusty-modbus --features full --example basic_client`

use std::time::Duration;

use rusty_modbus::client::ClientConfig;
use rusty_modbus::client::ModbusClient;
use rusty_modbus::types::UnitId;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:5020".parse()?;

    let config = ClientConfig {
        timeout: Duration::from_secs(5),
        ..ClientConfig::default()
    };

    let client = ModbusClient::connect(addr, config).await?;
    println!("Connected to {addr}");

    let registers = client.read_holding_registers(UnitId(1), 0, 10).await?;

    for (i, &val) in registers.iter().enumerate() {
        println!("Register {i:>3}: {val:#06X} ({val})");
    }

    client.shutdown().await;
    Ok(())
}
