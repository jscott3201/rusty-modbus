//! End-to-end tests for the CLI — exercises commands against a simulator.
//!
//! These tests call the command handlers directly rather than spawning a process,
//! which is faster and avoids binary path issues in CI.

use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_sim::{ModbusSimulator, generic_io};
use rusty_modbus_types::UnitId;

async fn start_sim() -> (ModbusSimulator, std::net::SocketAddr) {
    let mut sim = ModbusSimulator::from_config(generic_io());
    let addr = sim.start().await.unwrap();
    (sim, addr)
}

fn config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    }
}

#[tokio::test]
async fn read_holding_registers_human() {
    let (mut sim, addr) = start_sim().await;
    sim.set_holding_register(0, 100);
    sim.set_holding_register(1, 200);

    let client = ModbusClient::connect(addr, config()).await.unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 2)
        .await
        .unwrap();
    assert_eq!(regs, vec![100, 200]);

    sim.stop().await;
}

#[tokio::test]
async fn write_single_register_via_client() {
    let (mut sim, addr) = start_sim().await;

    let client = ModbusClient::connect(addr, config()).await.unwrap();
    client
        .write_single_register(UnitId(1), 0, 42)
        .await
        .unwrap();

    let regs = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![42]);

    sim.stop().await;
}

#[tokio::test]
async fn write_multiple_registers_via_client() {
    let (mut sim, addr) = start_sim().await;

    let client = ModbusClient::connect(addr, config()).await.unwrap();
    client
        .write_multiple_registers(UnitId(1), 0, &[10, 20, 30])
        .await
        .unwrap();

    let regs = client
        .read_holding_registers(UnitId(1), 0, 3)
        .await
        .unwrap();
    assert_eq!(regs, vec![10, 20, 30]);

    sim.stop().await;
}

#[tokio::test]
async fn read_coils_via_client() {
    let (mut sim, addr) = start_sim().await;
    sim.set_coil(0, true);
    sim.set_coil(1, false);
    sim.set_coil(2, true);

    let client = ModbusClient::connect(addr, config()).await.unwrap();
    let coils = client.read_coils(UnitId(1), 0, 3).await.unwrap();
    assert_eq!(coils, vec![true, false, true]);

    sim.stop().await;
}

#[tokio::test]
async fn write_coil_via_client() {
    let (mut sim, addr) = start_sim().await;

    let client = ModbusClient::connect(addr, config()).await.unwrap();
    client.write_single_coil(UnitId(1), 5, true).await.unwrap();

    let coils = client.read_coils(UnitId(1), 5, 1).await.unwrap();
    assert_eq!(coils, vec![true]);

    sim.stop().await;
}

#[tokio::test]
async fn read_input_registers_via_client() {
    let (mut sim, addr) = start_sim().await;
    sim.set_input_register(0, 999);

    let client = ModbusClient::connect(addr, config()).await.unwrap();
    let regs = client.read_input_registers(UnitId(1), 0, 1).await.unwrap();
    assert_eq!(regs, vec![999]);

    sim.stop().await;
}

#[tokio::test]
async fn connection_refused_returns_error() {
    // Connecting to a port with no server should fail.
    let result = ModbusClient::connect(
        "127.0.0.1:1".parse().unwrap(),
        ClientConfig {
            timeout: Duration::from_millis(500),
            ..ClientConfig::default()
        },
    )
    .await;
    assert!(result.is_err());
}
