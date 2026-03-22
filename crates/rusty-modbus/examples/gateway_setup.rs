//! Gateway example — TCP ↔ RTU-over-TCP bridge.
//!
//! Run: `cargo run -p rusty-modbus --features full --example gateway_setup`

use std::time::Duration;

use rusty_modbus::gateway::{GatewayConfig, ModbusGateway, RouteEntry};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = GatewayConfig {
        tcp_listen: "0.0.0.0:5020".parse()?,
        routes: vec![
            RouteEntry {
                unit_id_range: 1..=10,
                backend_addr: "192.168.1.100:5020".parse()?,
            },
            RouteEntry {
                unit_id_range: 11..=20,
                backend_addr: "192.168.1.101:5020".parse()?,
            },
        ],
        serial_timeout: Duration::from_secs(1),
        ..GatewayConfig::default()
    };

    let gateway = ModbusGateway::start(config).await?;
    println!("Gateway on {}", gateway.local_addr());
    println!("  Unit 1-10  → 192.168.1.100:5020");
    println!("  Unit 11-20 → 192.168.1.101:5020");
    println!("Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;
    gateway.stop().await;
    Ok(())
}
