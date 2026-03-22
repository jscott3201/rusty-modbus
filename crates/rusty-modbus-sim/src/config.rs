//! YAML-based simulator device configuration.

use serde::{Deserialize, Serialize};

/// Top-level simulator configuration (deserializable from YAML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimConfig {
    /// Device identity.
    pub device: DeviceConfig,
    /// Register definitions.
    #[serde(default)]
    pub registers: RegisterConfig,
    /// Fault injection rules.
    #[serde(default)]
    pub faults: Vec<FaultConfig>,
}

/// Device identity and settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Modbus unit ID. Default: 1.
    #[serde(default = "default_unit_id")]
    pub unit_id: u8,
    /// Vendor name (for device identification FC 0x2B).
    #[serde(default = "default_vendor")]
    pub vendor_name: String,
    /// Product code.
    #[serde(default = "default_product")]
    pub product_code: String,
    /// Firmware revision.
    #[serde(default = "default_revision")]
    pub revision: String,
    /// Listen address. Default: `127.0.0.1:0` (ephemeral port).
    #[serde(default = "default_listen")]
    pub listen_addr: String,
}

fn default_unit_id() -> u8 { 1 }
fn default_vendor() -> String { String::from("rusty-modbus-sim") }
fn default_product() -> String { String::from("SIM") }
fn default_revision() -> String { String::from("0.1.0") }
fn default_listen() -> String { String::from("127.0.0.1:0") }

/// Register definitions for all four data tables.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegisterConfig {
    /// Holding register blocks.
    #[serde(default)]
    pub holding: Vec<RegisterBlock>,
    /// Input register blocks.
    #[serde(default)]
    pub input: Vec<RegisterBlock>,
    /// Coil blocks.
    #[serde(default)]
    pub coils: Vec<CoilBlock>,
    /// Discrete input blocks.
    #[serde(default)]
    pub discrete_inputs: Vec<CoilBlock>,
}

/// A contiguous block of registers with initial values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterBlock {
    /// Starting address.
    pub address: u16,
    /// Number of registers in this block.
    pub count: u16,
    /// Initial values (padded with 0 if shorter than count).
    #[serde(default)]
    pub initial: Vec<u16>,
    /// Update mode for dynamic simulation.
    #[serde(default)]
    pub mode: UpdateMode,
    /// Minimum value for random mode.
    #[serde(default)]
    pub min: u16,
    /// Maximum value for random mode.
    #[serde(default = "default_max_u16")]
    pub max: u16,
}

fn default_max_u16() -> u16 { 1000 }

/// A contiguous block of coils/discrete inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoilBlock {
    /// Starting address.
    pub address: u16,
    /// Number of coils.
    pub count: u16,
    /// Initial values (padded with `false` if shorter).
    #[serde(default)]
    pub initial: Vec<bool>,
}

/// How register values update between reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Values are static — only change via writes.
    #[default]
    Static,
    /// Values are randomized within `[min, max]` on each read.
    Random,
    /// Values increment by 1 on each read, wrapping at `max`.
    Increment,
}

/// Fault injection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultConfig {
    /// Type of fault to inject.
    #[serde(rename = "type")]
    pub fault_type: FaultType,
    /// Trigger condition — when to inject the fault.
    #[serde(default)]
    pub trigger: FaultTrigger,
    /// Exception code to return (for `exception` type).
    #[serde(default)]
    pub exception: Option<String>,
    /// Delay in milliseconds (for `delay` type).
    #[serde(default)]
    pub delay_ms: Option<u64>,
    /// Probability of fault occurring (0.0–1.0, for `corrupt` type).
    #[serde(default)]
    pub probability: Option<f64>,
}

/// Fault type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultType {
    /// Return an exception response.
    Exception,
    /// Add artificial latency.
    Delay,
    /// Drop the response entirely (simulate timeout).
    Timeout,
    /// Corrupt CRC (RTU only).
    Corrupt,
}

/// Trigger condition for fault injection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FaultTrigger {
    /// Match a specific function code name (e.g., `read_holding_registers`).
    pub function: Option<String>,
    /// Match a specific address.
    pub address: Option<u16>,
    /// Match a specific unit ID.
    pub unit_id: Option<u8>,
}
