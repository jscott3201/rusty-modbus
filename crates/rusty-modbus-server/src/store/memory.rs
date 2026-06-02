//! In-memory data store backed by flat arrays with `RwLock` protection.

use std::collections::HashMap;

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
    /// File records keyed by file number; each `Vec<u16>` is indexed by record
    /// number (FC 0x14 / 0x15). Files grow lazily on write.
    files: RwLock<HashMap<u16, Vec<u16>>>,
    /// FIFO queues keyed by pointer address (FC 0x18).
    fifo_queues: RwLock<HashMap<u16, Vec<u16>>>,
    /// Eight exception-status coils packed into one byte (FC 0x07).
    exception_status: RwLock<u8>,
    /// Device-specific server-identification blob (FC 0x11).
    server_id: RwLock<Vec<u8>>,
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
            files: RwLock::new(HashMap::new()),
            fifo_queues: RwLock::new(HashMap::new()),
            exception_status: RwLock::new(0),
            server_id: RwLock::new(b"rusty-modbus\xFF".to_vec()),
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

    /// Seed a single file-record register (for test/app setup). The file and
    /// record grow lazily so sparse records can be set in any order.
    pub fn set_file_record(&self, file_number: u16, record_number: u16, value: u16) {
        let mut files = self.files.write();
        let file = files.entry(file_number).or_default();
        let idx = usize::from(record_number);
        if idx >= file.len() {
            file.resize(idx + 1, 0);
        }
        file[idx] = value;
    }

    /// Seed a FIFO queue at `address` with `values` (for test/app setup).
    pub fn set_fifo_queue(&self, address: u16, values: Vec<u16>) {
        self.fifo_queues.write().insert(address, values);
    }

    /// Set the eight exception-status coils (FC 0x07) as one packed byte.
    pub fn set_exception_status(&self, status: u8) {
        *self.exception_status.write() = status;
    }

    /// Set the device-specific server-identification blob (FC 0x11).
    pub fn set_server_id(&self, data: Vec<u8>) {
        *self.server_id.write() = data;
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

fn check_range(address: u16, quantity: usize, max: usize) -> Result<(), ExceptionCode> {
    let end = usize::from(address)
        .checked_add(quantity)
        .ok_or(ExceptionCode::IllegalDataAddress)?;
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
        check_range(address, usize::from(quantity), coils.len())?;
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
        check_range(address, values.len(), coils.len())?;
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
        check_range(address, usize::from(quantity), inputs.len())?;
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
        check_range(address, usize::from(quantity), regs.len())?;
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
        check_range(address, values.len(), regs.len())?;
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
        check_range(address, usize::from(quantity), regs.len())?;
        let start = address as usize;
        let qty = quantity as usize;
        buf[..qty].copy_from_slice(&regs[start..start + qty]);
        Ok(qty)
    }

    async fn read_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        record_length: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let files = self.files.read();
        let file = files
            .get(&file_number)
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        let start = usize::from(record_number);
        let len = usize::from(record_length);
        let end = start
            .checked_add(len)
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        // The record range must exist in the file, and must fit the caller's
        // scratch buffer (the handler caps a single sub-response at the PDU limit).
        if end > file.len() || len > buf.len() {
            return Err(ExceptionCode::IllegalDataAddress);
        }
        buf[..len].copy_from_slice(&file[start..end]);
        Ok(len)
    }

    async fn write_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        values: &[u16],
    ) -> Result<(), ExceptionCode> {
        let mut files = self.files.write();
        let file = files.entry(file_number).or_default();
        let start = usize::from(record_number);
        let end = start
            .checked_add(values.len())
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        // Scratch-pad semantics: an in-memory store auto-creates and extends
        // files on write.
        if end > file.len() {
            file.resize(end, 0);
        }
        file[start..end].copy_from_slice(values);
        Ok(())
    }

    async fn read_fifo_queue(&self, address: u16) -> Result<Vec<u16>, ExceptionCode> {
        // `.cloned()` honors the non-destructive read contract (§6.18).
        self.fifo_queues
            .read()
            .get(&address)
            .cloned()
            .ok_or(ExceptionCode::IllegalDataAddress)
    }

    async fn read_exception_status(&self) -> Result<u8, ExceptionCode> {
        Ok(*self.exception_status.read())
    }

    async fn report_server_id(&self) -> Result<Vec<u8>, ExceptionCode> {
        Ok(self.server_id.read().clone())
    }
}
