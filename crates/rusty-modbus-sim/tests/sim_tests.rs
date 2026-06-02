//! Integration tests for the Modbus simulator.

use std::time::Duration;

use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_sim::config::{CoilBlock, RegisterBlock, UpdateMode};
use rusty_modbus_sim::{
    ModbusSimulator, SimError, generic_io, hvac_controller, power_meter, vfd_drive,
};
use rusty_modbus_types::UnitId;

fn client_config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    }
}

#[tokio::test]
async fn from_config_and_read_holding_registers() {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();

    // generic_io initializes holding registers 0..16 to 0.
    let client = ModbusClient::connect(addr, client_config()).await.unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 4)
        .await
        .unwrap();
    assert_eq!(regs, vec![0, 0, 0, 0]);

    sim.stop().await;
}

#[tokio::test]
async fn write_and_read_back() {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    client
        .write_multiple_registers(UnitId(1), 0, &[0xAA, 0xBB, 0xCC])
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 3)
        .await
        .unwrap();
    assert_eq!(regs, vec![0xAA, 0xBB, 0xCC]);

    sim.stop().await;
}

#[tokio::test]
async fn runtime_register_update() {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Update register programmatically.
    sim.set_holding_register(5, 0x1234);
    let regs = client
        .read_holding_registers(UnitId(1), 5, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0x1234]);

    sim.stop().await;
}

#[tokio::test]
async fn from_yaml_config() {
    let yaml = r"
device:
  unit_id: 1
  vendor_name: TestDevice
  product_code: TD-1
  revision: '1.0'
registers:
  holding:
    - address: 0
      count: 4
      initial: [100, 200, 300, 400]
faults: []
";
    let mut sim = ModbusSimulator::from_yaml(yaml).unwrap();
    let addr = sim.start().await.unwrap();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 4)
        .await
        .unwrap();
    assert_eq!(regs, vec![100, 200, 300, 400]);

    sim.stop().await;
}

#[tokio::test]
async fn hvac_profile_has_registers() {
    let mut sim = ModbusSimulator::from_config(hvac_controller()).unwrap();
    let addr = sim.start().await.unwrap();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // HVAC holding register 0 = setpoint 720 (72.0°F ×10).
    let regs = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![720]);

    // Read coils — first coil should be true (fan on).
    let coils = client.read_coils(UnitId(1), 0, 3).await.unwrap();
    assert_eq!(coils, vec![true, false, true]);

    sim.stop().await;
}

#[tokio::test]
async fn power_meter_profile_has_registers() {
    let mut sim = ModbusSimulator::from_config(power_meter()).unwrap();
    let addr = sim.start().await.unwrap();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // Power meter input register 0 = voltage 2400 (240.0V).
    let regs = client.read_input_registers(UnitId(2), 0, 1).await.unwrap();
    assert_eq!(regs, vec![2400]);

    sim.stop().await;
}

#[tokio::test]
async fn vfd_profile_has_registers() {
    let mut sim = ModbusSimulator::from_config(vfd_drive()).unwrap();
    let addr = sim.start().await.unwrap();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    // VFD holding register 0 = speed setpoint 1500 RPM.
    let regs = client
        .read_holding_registers(UnitId(3), 0, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![1500]);

    sim.stop().await;
}

#[tokio::test]
async fn coil_read_write() {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();

    client.write_single_coil(UnitId(1), 3, true).await.unwrap();
    let coils = client.read_coils(UnitId(1), 3, 1).await.unwrap();
    assert_eq!(coils, vec![true]);

    sim.stop().await;
}

#[tokio::test]
async fn set_input_register_at_runtime() {
    let mut sim = ModbusSimulator::from_config(generic_io()).unwrap();
    let addr = sim.start().await.unwrap();

    sim.set_input_register(0, 0x5555);

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();
    let regs = client.read_input_registers(UnitId(1), 0, 1).await.unwrap();
    assert_eq!(regs, vec![0x5555]);

    sim.stop().await;
}

#[tokio::test]
async fn from_config_allows_single_register_at_last_address() {
    let mut config = generic_io();
    config.registers.holding = vec![RegisterBlock {
        address: u16::MAX,
        count: 1,
        initial: vec![0xBEEF],
        mode: UpdateMode::Static,
        min: 0,
        max: 0,
    }];

    let mut sim = ModbusSimulator::from_config(config).unwrap();
    let addr = sim.start().await.unwrap();

    let client = ModbusClient::connect(addr, client_config()).await.unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), u16::MAX, 1)
        .await
        .unwrap();
    assert_eq!(regs, vec![0xBEEF]);

    sim.stop().await;
}

#[test]
fn from_config_rejects_register_block_that_overflows_address_space() {
    let mut config = generic_io();
    config.registers.holding = vec![RegisterBlock {
        address: u16::MAX,
        count: 2,
        initial: vec![0x1111, 0x2222],
        mode: UpdateMode::Static,
        min: 0,
        max: 0,
    }];

    let err = ModbusSimulator::from_config(config).unwrap_err();

    assert!(matches!(err, SimError::Config(message) if message.contains("holding")));
}

#[test]
fn from_yaml_rejects_register_block_that_overflows_address_space() {
    let yaml = r"
device:
  unit_id: 1
registers:
  holding:
    - address: 65535
      count: 2
      initial: [1, 2]
faults: []
";

    let err = ModbusSimulator::from_yaml(yaml).unwrap_err();

    assert!(matches!(err, SimError::Config(message) if message.contains("holding")));
}

#[test]
fn from_config_rejects_coil_block_that_overflows_address_space() {
    let mut config = generic_io();
    config.registers.coils = vec![CoilBlock {
        address: u16::MAX,
        count: 2,
        initial: vec![true, false],
    }];

    let err = ModbusSimulator::from_config(config).unwrap_err();

    assert!(matches!(err, SimError::Config(message) if message.contains("coils")));
}
