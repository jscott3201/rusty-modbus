//! In-memory data store backed by flat arrays with `RwLock` protection.

use parking_lot::RwLock;
use rusty_modbus_types::ExceptionCode;

use super::DataStore;

/// Configuration for the in-memory data store.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Number of coils (address space size). Default: 65536.
    pub coil_count: usize,
    /// Number of discrete inputs. Default: 65536.
    pub discrete_input_count: usize,
    /// Number of holding registers. Default: 65536.
    pub holding_register_count: usize,
    /// Number of input registers. Default: 65536.
    pub input_register_count: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            coil_count: 65536,
            discrete_input_count: 65536,
            holding_register_count: 65536,
            input_register_count: 65536,
        }
    }
}

/// In-memory data store using flat `Vec`s with `RwLock` protection.
pub struct InMemoryStore {
    coils: RwLock<Vec<bool>>,
    discrete_inputs: RwLock<Vec<bool>>,
    holding_registers: RwLock<Vec<u16>>,
    input_registers: RwLock<Vec<u16>>,
}

impl InMemoryStore {
    /// Create a new in-memory store with the given address space sizes.
    #[must_use]
    pub fn new(config: StoreConfig) -> Self {
        Self {
            coils: RwLock::new(vec![false; config.coil_count]),
            discrete_inputs: RwLock::new(vec![false; config.discrete_input_count]),
            holding_registers: RwLock::new(vec![0u16; config.holding_register_count]),
            input_registers: RwLock::new(vec![0u16; config.input_register_count]),
        }
    }

    /// Direct write to an input register (for application-level updates).
    pub fn set_input_register(&self, address: u16, value: u16) {
        let mut regs = self.input_registers.write();
        if (address as usize) < regs.len() {
            regs[address as usize] = value;
        }
    }

    /// Direct write to a discrete input (for application-level updates).
    pub fn set_discrete_input(&self, address: u16, value: bool) {
        let mut inputs = self.discrete_inputs.write();
        if (address as usize) < inputs.len() {
            inputs[address as usize] = value;
        }
    }

    /// Direct write to a holding register (for test setup).
    pub fn set_holding_register(&self, address: u16, value: u16) {
        let mut regs = self.holding_registers.write();
        if (address as usize) < regs.len() {
            regs[address as usize] = value;
        }
    }

    /// Direct write to a coil (for test setup).
    pub fn set_coil(&self, address: u16, value: bool) {
        let mut coils = self.coils.write();
        if (address as usize) < coils.len() {
            coils[address as usize] = value;
        }
    }
}

impl std::fmt::Debug for InMemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryStore")
            .field("coils", &self.coils.read().len())
            .field("holding_registers", &self.holding_registers.read().len())
            .finish_non_exhaustive()
    }
}

fn check_range(address: u16, quantity: u16, max: usize) -> Result<(), ExceptionCode> {
    let end = usize::from(address) + usize::from(quantity);
    if end > max {
        return Err(ExceptionCode::IllegalDataAddress);
    }
    Ok(())
}

impl DataStore for InMemoryStore {
    async fn read_coils(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        let coils = self.coils.read();
        check_range(address, quantity, coils.len())?;
        let start = address as usize;
        let qty = quantity as usize;
        buf[..qty].copy_from_slice(&coils[start..start + qty]);
        Ok(qty)
    }

    async fn write_coil(&self, address: u16, value: bool) -> Result<(), ExceptionCode> {
        let mut coils = self.coils.write();
        check_range(address, 1, coils.len())?;
        coils[address as usize] = value;
        Ok(())
    }

    async fn write_coils(&self, address: u16, values: &[bool]) -> Result<(), ExceptionCode> {
        let mut coils = self.coils.write();
        let qty = u16::try_from(values.len()).unwrap_or(u16::MAX);
        check_range(address, qty, coils.len())?;
        let start = address as usize;
        coils[start..start + values.len()].copy_from_slice(values);
        Ok(())
    }

    async fn read_discrete_inputs(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        let inputs = self.discrete_inputs.read();
        check_range(address, quantity, inputs.len())?;
        let start = address as usize;
        let qty = quantity as usize;
        buf[..qty].copy_from_slice(&inputs[start..start + qty]);
        Ok(qty)
    }

    async fn read_holding_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let regs = self.holding_registers.read();
        check_range(address, quantity, regs.len())?;
        let start = address as usize;
        let qty = quantity as usize;
        buf[..qty].copy_from_slice(&regs[start..start + qty]);
        Ok(qty)
    }

    async fn write_register(&self, address: u16, value: u16) -> Result<(), ExceptionCode> {
        let mut regs = self.holding_registers.write();
        check_range(address, 1, regs.len())?;
        regs[address as usize] = value;
        Ok(())
    }

    async fn write_registers(&self, address: u16, values: &[u16]) -> Result<(), ExceptionCode> {
        let mut regs = self.holding_registers.write();
        let qty = u16::try_from(values.len()).unwrap_or(u16::MAX);
        check_range(address, qty, regs.len())?;
        let start = address as usize;
        regs[start..start + values.len()].copy_from_slice(values);
        Ok(())
    }

    async fn read_input_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let regs = self.input_registers.read();
        check_range(address, quantity, regs.len())?;
        let start = address as usize;
        let qty = quantity as usize;
        buf[..qty].copy_from_slice(&regs[start..start + qty]);
        Ok(qty)
    }
}
