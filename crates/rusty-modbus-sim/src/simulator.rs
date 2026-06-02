//! `ModbusSimulator` — wraps `ModbusServer` with YAML-configurable register maps.

use std::net::SocketAddr;
use std::sync::Arc;

use rusty_modbus_server::ModbusServer;
use rusty_modbus_server::config::{DeviceIdentification, ServerConfig};
use rusty_modbus_server::store::memory::{InMemoryStore, StoreConfig};
use rusty_modbus_types::UnitId;

use crate::config::{CoilBlock, RegisterBlock, RegisterConfig, SimConfig};
use crate::error::SimError;

const MODBUS_ADDRESS_SPACE: usize = 65_536;

/// Device simulator wrapping a `ModbusServer` with preconfigured register maps.
pub struct ModbusSimulator {
    config: SimConfig,
    store: Arc<InMemoryStore>,
    server: Option<ModbusServer<InMemoryStore>>,
}

impl ModbusSimulator {
    /// Create a simulator from a YAML configuration string.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::ConfigParse`] if the YAML is invalid.
    pub fn from_yaml(yaml: &str) -> Result<Self, SimError> {
        let config: SimConfig = serde_yaml_ng::from_str(yaml).map_err(SimError::ConfigParse)?;
        Self::from_config(config)
    }

    /// Create a simulator from a programmatic configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::Config`] if any configured block exceeds the Modbus
    /// 16-bit address space.
    pub fn from_config(config: SimConfig) -> Result<Self, SimError> {
        validate_register_config(&config.registers)?;

        let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
        apply_register_config(&store, &config.registers);

        Ok(Self {
            config,
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
        let listen_addr: SocketAddr = self
            .config
            .device
            .listen_addr
            .parse()
            .map_err(|e| SimError::Config(format!("invalid listen address: {e}")))?;

        let server_config = ServerConfig {
            listen_addr,
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
    pub fn set_holding_register(&self, address: u16, value: u16) {
        self.store.set_holding_register(address, value);
    }

    /// Update an input register at runtime.
    pub fn set_input_register(&self, address: u16, value: u16) {
        self.store.set_input_register(address, value);
    }

    /// Update a coil at runtime.
    pub fn set_coil(&self, address: u16, value: bool) {
        self.store.set_coil(address, value);
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

fn validate_register_config(config: &RegisterConfig) -> Result<(), SimError> {
    for block in &config.holding {
        check_block_range("holding", block.address, block.count)?;
    }
    for block in &config.input {
        check_block_range("input", block.address, block.count)?;
    }
    for block in &config.coils {
        check_block_range("coils", block.address, block.count)?;
    }
    for block in &config.discrete_inputs {
        check_block_range("discrete_inputs", block.address, block.count)?;
    }
    Ok(())
}

fn check_block_range(kind: &str, address: u16, count: u16) -> Result<(), SimError> {
    let end = usize::from(address) + usize::from(count);
    if end <= MODBUS_ADDRESS_SPACE {
        Ok(())
    } else {
        Err(SimError::Config(format!(
            "{kind} block at address {address} with count {count} exceeds Modbus address space"
        )))
    }
}

/// Apply register configuration to the in-memory store.
fn apply_register_config(store: &InMemoryStore, config: &RegisterConfig) {
    apply_register_blocks(&config.holding, |address, value| {
        store.set_holding_register(address, value);
    });
    apply_register_blocks(&config.input, |address, value| {
        store.set_input_register(address, value);
    });
    apply_coil_blocks(&config.coils, |address, value| {
        store.set_coil(address, value);
    });
    apply_coil_blocks(&config.discrete_inputs, |address, value| {
        store.set_discrete_input(address, value);
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
