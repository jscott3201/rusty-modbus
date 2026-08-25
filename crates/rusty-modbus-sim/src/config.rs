//! YAML-based simulator device configuration.

use serde::{Deserialize, Serialize};

/// Top-level simulator configuration (deserializable from YAML).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimConfig {
    /// Device identity.
    pub device: DeviceConfig,
    /// Register definitions.
    #[serde(default)]
    pub registers: RegisterConfig,
    /// Fault injection rules. Nonempty lists are rejected until fault injection is implemented.
    #[serde(default)]
    pub faults: Vec<FaultConfig>,
}

/// Device identity and settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

fn default_unit_id() -> u8 {
    1
}
fn default_vendor() -> String {
    String::from("rusty-modbus-sim")
}
fn default_product() -> String {
    String::from("SIM")
}
fn default_revision() -> String {
    String::from(env!("CARGO_PKG_VERSION"))
}
fn default_listen() -> String {
    String::from("127.0.0.1:0")
}

/// Register definitions for all four data tables.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct RegisterBlock {
    /// Starting address.
    pub address: u16,
    /// Number of registers in this block.
    pub count: u16,
    /// Initial values (padded with 0 if shorter than count).
    #[serde(default)]
    pub initial: Vec<u16>,
    /// Requested update mode. Only [`UpdateMode::Static`] is currently accepted.
    #[serde(default)]
    pub mode: UpdateMode,
    /// Reserved lower bound. Static blocks must use 0.
    #[serde(default)]
    pub min: u16,
    /// Reserved upper bound. Static blocks must use 65535.
    #[serde(default = "default_max_u16")]
    pub max: u16,
}

fn default_max_u16() -> u16 {
    u16::MAX
}

/// A contiguous block of coils/discrete inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Values do not update automatically.
    #[default]
    Static,
    /// Reserved for future randomized updates; rejected by current validation.
    Random,
    /// Reserved for future incrementing updates; rejected by current validation.
    Increment,
}

/// Reserved fault injection configuration. Current validation rejects every entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct FaultTrigger {
    /// Match a specific function code name (e.g., `read_holding_registers`).
    pub function: Option<String>,
    /// Match a specific address.
    pub address: Option<u16>,
    /// Match a specific unit ID.
    pub unit_id: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::DeviceConfig;

    #[test]
    fn omitted_revision_uses_package_version() {
        let config: DeviceConfig = serde_yaml_ng::from_str("{}").unwrap();
        assert_eq!(config.revision, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn configured_revision_is_preserved() {
        let config: DeviceConfig = serde_yaml_ng::from_str("revision: device-firmware-7").unwrap();
        assert_eq!(config.revision, "device-firmware-7");
    }
}
