//! `ModbusSimulator` — wraps `ModbusServer` with YAML-configurable register maps.

use std::net::SocketAddr;
use std::sync::Arc;

use rusty_modbus_server::ModbusServer;
use rusty_modbus_server::config::{DeviceIdentification, ServerConfig};
use rusty_modbus_server::store::memory::{InMemoryStore, StoreConfig};
use rusty_modbus_types::UnitId;

use crate::config::{CoilBlock, RegisterBlock, RegisterConfig, SimConfig};
use crate::error::SimError;

/// Device simulator wrapping a `ModbusServer` with preconfigured register maps.
pub struct ModbusSimulator {
    config: SimConfig,
    listen_addr: SocketAddr,
    store: Arc<InMemoryStore>,
    server: Option<ModbusServer<InMemoryStore>>,
}

impl ModbusSimulator {
    /// Create a simulator from a YAML configuration string.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::ConfigParse`] if the YAML is invalid, or
    /// [`SimError::Config`] if the parsed configuration is unsupported or
    /// violates a simulator invariant.
    pub fn from_yaml(yaml: &str) -> Result<Self, SimError> {
        let config: SimConfig = serde_yaml_ng::from_str(yaml).map_err(SimError::ConfigParse)?;
        Self::from_config(config)
    }

    /// Create a simulator from a programmatic configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::Config`] before allocating runtime state if the
    /// configuration is unsupported or violates a simulator invariant.
    pub fn from_config(config: SimConfig) -> Result<Self, SimError> {
        let listen_addr = validate_config(&config)?;

        let store = Arc::new(InMemoryStore::try_new(StoreConfig::default())?);
        apply_register_config(&store, &config.registers);

        Ok(Self {
            config,
            listen_addr,
            store,
            server: None,
        })
    }

    /// Start the simulator server. Returns the bound address.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::Server`] if the server fails to start.
    pub async fn start(&mut self) -> Result<SocketAddr, SimError> {
        let server_config = ServerConfig {
            listen_addr: self.listen_addr,
            unit_id: UnitId(self.config.device.unit_id),
            device_id: DeviceIdentification {
                vendor_name: self.config.device.vendor_name.clone(),
                product_code: self.config.device.product_code.clone(),
                major_minor_revision: self.config.device.revision.clone(),
                ..DeviceIdentification::default()
            },
            ..ServerConfig::default()
        };

        let server = ModbusServer::start(server_config, Arc::clone(&self.store))
            .await
            .map_err(SimError::Server)?;

        let addr = server.local_addr();
        self.server = Some(server);
        Ok(addr)
    }

    /// Stop the simulator server.
    pub async fn stop(&mut self) {
        if let Some(server) = &self.server {
            server.stop().await;
        }
        self.server = None;
    }

    /// Update a holding register at runtime.
    pub fn set_holding_register(&self, address: u16, value: u16) -> Result<(), SimError> {
        self.store
            .set_holding_register(address, value)
            .map_err(SimError::Store)
    }

    /// Update an input register at runtime.
    pub fn set_input_register(&self, address: u16, value: u16) -> Result<(), SimError> {
        self.store
            .set_input_register(address, value)
            .map_err(SimError::Store)
    }

    /// Update a coil at runtime.
    pub fn set_coil(&self, address: u16, value: bool) -> Result<(), SimError> {
        self.store.set_coil(address, value).map_err(SimError::Store)
    }

    /// Unit Identifier accepted by this simulator.
    #[must_use]
    pub fn unit_id(&self) -> UnitId {
        UnitId(self.config.device.unit_id)
    }

    /// Get the bound address (only valid after `start()`).
    ///
    /// # Panics
    ///
    /// Panics if the server hasn't been started.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.server
            .as_ref()
            .expect("simulator not started")
            .local_addr()
    }
}

impl std::fmt::Debug for ModbusSimulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModbusSimulator")
            .field("unit_id", &self.config.device.unit_id)
            .field("running", &self.server.is_some())
            .finish_non_exhaustive()
    }
}

fn validate_config(config: &SimConfig) -> Result<SocketAddr, SimError> {
    let unit_id = UnitId(config.device.unit_id);
    if !unit_id.is_valid_slave() && !unit_id.is_tcp_device() {
        return Err(SimError::Config(format!(
            "device.unit_id must be 1..=247 or 255, got {}",
            config.device.unit_id
        )));
    }

    let listen_addr = config.device.listen_addr.parse().map_err(|error| {
        SimError::Config(format!(
            "invalid device.listen_addr {:?}: {error}",
            config.device.listen_addr
        ))
    })?;

    if !config.faults.is_empty() {
        return Err(SimError::Config(String::from(
            "faults are unsupported; remove all fault entries",
        )));
    }

    validate_register_config(&config.registers)?;
    Ok(listen_addr)
}

fn validate_register_config(config: &RegisterConfig) -> Result<(), SimError> {
    validate_block_layout("holding", &config.holding, |block| {
        (block.address, block.count, block.initial.len())
    })?;
    validate_register_behavior("holding", &config.holding)?;

    validate_block_layout("input", &config.input, |block| {
        (block.address, block.count, block.initial.len())
    })?;
    validate_register_behavior("input", &config.input)?;

    validate_block_layout("coils", &config.coils, |block| {
        (block.address, block.count, block.initial.len())
    })?;
    validate_block_layout("discrete_inputs", &config.discrete_inputs, |block| {
        (block.address, block.count, block.initial.len())
    })?;
    Ok(())
}

fn validate_register_behavior(table: &str, blocks: &[RegisterBlock]) -> Result<(), SimError> {
    for (index, block) in blocks.iter().enumerate() {
        if !matches!(block.mode, crate::config::UpdateMode::Static) {
            return Err(SimError::Config(format!(
                "registers.{table}[{index}].mode is unsupported; only static is supported"
            )));
        }
        if block.min != 0 || block.max != u16::MAX {
            return Err(SimError::Config(format!(
                "registers.{table}[{index}] static min/max must be 0/65535, got {}/{}",
                block.min, block.max
            )));
        }
    }
    Ok(())
}

fn validate_block_layout<T>(
    table: &str,
    blocks: &[T],
    fields: impl Fn(&T) -> (u16, u16, usize) + Copy,
) -> Result<(), SimError> {
    let mut occupied = [0_u64; 1024];
    for (index, block) in blocks.iter().enumerate() {
        let (address, count, initial_len) = fields(block);
        if count == 0 {
            return Err(SimError::Config(format!(
                "registers.{table}[{index}].count must be nonzero"
            )));
        }
        if address.checked_add(count - 1).is_none() {
            return Err(SimError::Config(format!(
                "registers.{table}[{index}] block at address {address} with count {count} exceeds Modbus address space"
            )));
        }
        if initial_len > usize::from(count) {
            return Err(SimError::Config(format!(
                "registers.{table}[{index}].initial has {initial_len} values but count is {count}"
            )));
        }

        let end = usize::from(address) + usize::from(count) - 1;
        if let Some(overlap) = (usize::from(address)..=end)
            .find(|&item| occupied[item / 64] & (1_u64 << (item % 64)) != 0)
        {
            let (previous_index, previous_start, previous_end) =
                blocks[..index]
                    .iter()
                    .enumerate()
                    .find_map(|(previous_index, previous)| {
                        let (previous_start, previous_count, _) = fields(previous);
                        let previous_end =
                            usize::from(previous_start) + usize::from(previous_count) - 1;
                        (usize::from(previous_start) <= overlap && overlap <= previous_end)
                            .then_some((previous_index, previous_start, previous_end))
                    })
                    .expect("occupied address must belong to a previous validated block");
            return Err(SimError::Config(format!(
                "registers.{table}[{previous_index}] range {previous_start}..={previous_end} overlaps registers.{table}[{index}] range {address}..={end}"
            )));
        }
        for item in usize::from(address)..=end {
            occupied[item / 64] |= 1_u64 << (item % 64);
        }
    }

    Ok(())
}

/// Apply register configuration to the in-memory store.
fn apply_register_config(store: &InMemoryStore, config: &RegisterConfig) {
    apply_register_blocks(&config.holding, |address, value| {
        store
            .set_holding_register(address, value)
            .expect("validated holding register config should fit store");
    });
    apply_register_blocks(&config.input, |address, value| {
        store
            .set_input_register(address, value)
            .expect("validated input register config should fit store");
    });
    apply_coil_blocks(&config.coils, |address, value| {
        store
            .set_coil(address, value)
            .expect("validated coil config should fit store");
    });
    apply_coil_blocks(&config.discrete_inputs, |address, value| {
        store
            .set_discrete_input(address, value)
            .expect("validated discrete input config should fit store");
    });
}

fn apply_register_blocks(blocks: &[RegisterBlock], mut set: impl FnMut(u16, u16)) {
    for block in blocks {
        for (i, &val) in block.initial.iter().enumerate() {
            if i < usize::from(block.count)
                && let Some(address) = offset_address(block.address, i)
            {
                set(address, val);
            }
        }
    }
}

fn apply_coil_blocks(blocks: &[CoilBlock], mut set: impl FnMut(u16, bool)) {
    for block in blocks {
        for (i, &val) in block.initial.iter().enumerate() {
            if i < usize::from(block.count)
                && let Some(address) = offset_address(block.address, i)
            {
                set(address, val);
            }
        }
    }
}

fn offset_address(address: u16, offset: usize) -> Option<u16> {
    let offset = u16::try_from(offset).ok()?;
    address.checked_add(offset)
}
