//! Simulator conformance tests.
//!
//! Verifies that pre-built device profiles produce valid Modbus servers
//! and that all data table operations work through the simulator.

use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_sim::{ModbusSimulator, generic_io, hvac_controller, power_meter, vfd_drive};
use rusty_modbus_types::UnitId;

fn config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    }
}

// ── Pre-built profiles start and respond ──────────────────────────

#[tokio::test]
async fn generic_io_profile_responds() {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();
    let client = ModbusClient::connect(addr, config()).await.unwrap();

    // generic_io: unit_id=1, 16 holding regs initialized to 0
    let regs = client
        .read_holding_registers(UnitId(1), 0, 16)
        .await
        .unwrap();
    assert_eq!(regs.len(), 16);
    assert!(regs.iter().all(|&r| r == 0));

    sim.stop().await;
}

#[tokio::test]
async fn hvac_controller_profile_has_setpoints() {
    let mut sim = ModbusSimulator::from_config(hvac_controller()).unwrap();
    let addr = sim.start().await.unwrap();
    let client = ModbusClient::connect(addr, config()).await.unwrap();

    // hvac_controller: unit_id=1, holding[0]=720 (setpoint 72.0°F)
    let regs = client
        .read_holding_registers(UnitId(1), 0, 3)
        .await
        .unwrap();
    assert_eq!(regs[0], 720); // setpoint
    assert_eq!(regs[1], 680); // heating setpoint
    assert_eq!(regs[2], 760); // cooling setpoint

    sim.stop().await;
}

#[tokio::test]
async fn power_meter_profile_has_input_registers() {
    let mut sim = ModbusSimulator::from_config(power_meter()).unwrap();
    let addr = sim.start().await.unwrap();
    let client = ModbusClient::connect(addr, config()).await.unwrap();

    // power_meter: unit_id=2, input[0]=2400 (240.0V)
    let regs = client.read_input_registers(UnitId(2), 0, 3).await.unwrap();
    assert_eq!(regs[0], 2400); // voltage L1
    assert_eq!(regs[1], 2390); // voltage L2
    assert_eq!(regs[2], 2410); // voltage L3

    sim.stop().await;
}

#[tokio::test]
async fn vfd_drive_profile_has_all_tables() {
    let mut sim = ModbusSimulator::from_config(vfd_drive()).unwrap();
    let addr = sim.start().await.unwrap();
    let client = ModbusClient::connect(addr, config()).await.unwrap();

    // vfd_drive: unit_id=3
    // Holding: speed setpoint=1500
    let regs = client
        .read_holding_registers(UnitId(3), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs[0], 1500);

    // Coils
    let coils = client.read_coils(UnitId(3), 0, 4).await.unwrap();
    assert_eq!(coils, vec![false, false, false, false]);

    // Discrete inputs
    let di = client.read_discrete_inputs(UnitId(3), 0, 4).await.unwrap();
    assert!(di[0]); // first DI is true per profile

    sim.stop().await;
}

// ── Runtime register updates ──────────────────────────────────────

#[tokio::test]
async fn simulator_runtime_update() {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();
    let client = ModbusClient::connect(addr, config()).await.unwrap();

    // Update at runtime
    sim.set_holding_register(0, 0xBEEF);
    sim.set_input_register(0, 0xCAFE);
    sim.set_coil(0, true);

    let regs = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0xBEEF]);

    let iregs = client.read_input_registers(UnitId(1), 0, 1).await.unwrap();
    assert_eq!(iregs, vec![0xCAFE]);

    let coils = client.read_coils(UnitId(1), 0, 1).await.unwrap();
    assert_eq!(coils, vec![true]);

    sim.stop().await;
}

// ── Write through simulator ───────────────────────────────────────

#[tokio::test]
async fn simulator_write_and_read_back() {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();
    let client = ModbusClient::connect(addr, config()).await.unwrap();

    // Write via client, read back
    client
        .write_single_register(UnitId(1), 5, 42)
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 5, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![42]);

    client.write_single_coil(UnitId(1), 3, true).await.unwrap();
    let coils = client.read_coils(UnitId(1), 3, 1).await.unwrap();
    assert_eq!(coils, vec![true]);

    sim.stop().await;
}
