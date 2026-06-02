//! End-to-end tests for the CLI — exercises commands against a simulator.
//!
//! These tests call the command handlers directly rather than spawning a process,
//! which is faster and avoids binary path issues in CI.

use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_sim::{ModbusSimulator, generic_io};
use rusty_modbus_types::UnitId;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time;

async fn start_sim() -> (ModbusSimulator, std::net::SocketAddr) {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
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
    sim.set_holding_register(0, 100).unwrap();
    sim.set_holding_register(1, 200).unwrap();

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
    sim.set_coil(0, true).unwrap();
    sim.set_coil(1, false).unwrap();
    sim.set_coil(2, true).unwrap();

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
    sim.set_input_register(0, 999).unwrap();

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

#[tokio::test]
async fn json_logging_goes_to_stderr_without_polluting_stdout() {
    let (mut sim, addr) = start_sim().await;
    sim.set_holding_register(0, 100).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_modbus"))
        .args([
            "--host",
            &addr.to_string(),
            "--unit-id",
            "1",
            "--timeout",
            "1",
            "--format",
            "json",
            "--log-filter",
            "rusty_modbus_client=debug,rusty_modbus_tcp=debug",
            "--log-format",
            "json",
            "read",
            "hr",
            "0",
            "1",
        ])
        .output()
        .await
        .unwrap();

    sim.stop().await;

    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(stdout["function"], "read_registers");
    assert_eq!(stdout["values"], serde_json::json!([100]));

    let stderr = String::from_utf8(output.stderr).unwrap();
    let first_log = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .expect("expected at least one structured log line");
    let log: Value = serde_json::from_str(first_log).unwrap();
    assert_eq!(log["level"], "DEBUG");
    assert!(
        log["target"]
            .as_str()
            .is_some_and(|target| target.starts_with("rusty_modbus_"))
    );
}

#[tokio::test]
async fn server_command_serves_seeded_memory_store() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_modbus"))
        .args([
            "--unit-id",
            "1",
            "server",
            "--listen",
            "127.0.0.1:0",
            "--holding",
            "0=0xBEEF",
            "--coil",
            "5=on",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stderr = child.stderr.take().unwrap();
    let mut lines = BufReader::new(stderr).lines();
    let startup = match time::timeout(Duration::from_secs(30), lines.next_line()).await {
        Ok(Ok(Some(line))) => line,
        Ok(Ok(None)) => {
            let _ = child.kill().await;
            panic!("server exited before writing startup line");
        }
        Ok(Err(error)) => {
            let _ = child.kill().await;
            panic!("failed to read server startup line: {error}");
        }
        Err(_) => {
            let _ = child.kill().await;
            panic!("timed out waiting for server startup line");
        }
    };
    let addr = parse_server_listen_addr(&startup);

    let client = ModbusClient::connect(addr, config()).await.unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    let coils = client.read_coils(UnitId(1), 5, 1).await.unwrap();
    client.shutdown().await;

    child.kill().await.unwrap();
    let _ = child.wait().await;

    assert_eq!(regs, vec![0xBEEF]);
    assert_eq!(coils, vec![true]);
}

fn parse_server_listen_addr(line: &str) -> SocketAddr {
    line.strip_prefix("Modbus server listening on ")
        .and_then(|rest| rest.split_once(" (unit ").map(|(addr, _)| addr))
        .expect("unexpected server startup line")
        .parse()
        .unwrap()
}
