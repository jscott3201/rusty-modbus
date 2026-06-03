//! In-memory data store backed by flat arrays with `RwLock` protection.

use std::collections::HashMap;

use parking_lot::RwLock;
use rusty_modbus_types::{DiagnosticSubFunction, ExceptionCode};

use crate::file_record::{self, MAX_RECORD_NUMBER, MIN_FILE_NUMBER, RECORD_COUNT};

use super::{
    DataStore, MAX_DIAGNOSTIC_RESPONSE_DATA_LEN, MAX_FILE_RECORD_REGISTERS, MAX_SERVER_ID_BYTES,
    bits::BitTable, pack_registers_be, validate_packed_coils, validate_register_values_be,
};

/// Maximum number of entries in any Modbus data table.
pub const MAX_TABLE_SIZE: usize = 65_536;

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

impl StoreConfig {
    /// Validate configured table sizes before allocating backing vectors.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::TableTooLarge`] if any table exceeds the 16-bit
    /// Modbus address space.
    pub fn validate(&self) -> Result<(), StoreError> {
        validate_table_size("coils", self.coil_count)?;
        validate_table_size("discrete_inputs", self.discrete_input_count)?;
        validate_table_size("holding_registers", self.holding_register_count)?;
        validate_table_size("input_registers", self.input_register_count)?;
        Ok(())
    }
}

/// Errors produced by in-memory store configuration and setup helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// A configured table has more entries than Modbus can address.
    #[error("{table} table size {count} exceeds Modbus address space ({max})")]
    TableTooLarge {
        /// Table name.
        table: &'static str,
        /// Requested item count.
        count: usize,
        /// Maximum supported item count.
        max: usize,
    },
    /// A setup helper addressed outside the configured table.
    #[error("{table} address {address} is outside configured table size {len}")]
    AddressOutOfRange {
        /// Table name.
        table: &'static str,
        /// Requested Modbus address.
        address: u16,
        /// Configured table length.
        len: usize,
    },
    /// A setup helper used file number 0, which is outside the Modbus range.
    #[error("file number {file_number} is outside Modbus file range ({minimum}..=65535)")]
    FileNumberOutOfRange {
        /// Requested file number.
        file_number: u16,
        /// Minimum valid file number.
        minimum: u16,
    },
    /// A setup helper used a file record outside the 10,000-record file range.
    #[error("file record {record_number} is outside Modbus file record range (0..={maximum})")]
    FileRecordOutOfRange {
        /// Requested record number.
        record_number: u16,
        /// Maximum valid record number.
        maximum: u16,
    },
}

/// In-memory data store using flat tables with `RwLock` protection.
pub struct InMemoryStore {
    coils: RwLock<BitTable>,
    discrete_inputs: RwLock<BitTable>,
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
    ///
    /// # Panics
    ///
    /// Panics if any table size exceeds the 16-bit Modbus address space. Use
    /// [`Self::try_new`] to handle invalid configuration without panicking.
    #[must_use]
    pub fn new(config: StoreConfig) -> Self {
        Self::try_new(config).expect("StoreConfig should fit the Modbus address space")
    }

    /// Create a new in-memory store with checked address-space sizes.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::TableTooLarge`] if any table size exceeds 65,536.
    pub fn try_new(config: StoreConfig) -> Result<Self, StoreError> {
        config.validate()?;
        Ok(Self {
            coils: RwLock::new(BitTable::new(config.coil_count)),
            discrete_inputs: RwLock::new(BitTable::new(config.discrete_input_count)),
            holding_registers: RwLock::new(vec![0u16; config.holding_register_count]),
            input_registers: RwLock::new(vec![0u16; config.input_register_count]),
            files: RwLock::new(HashMap::new()),
            fifo_queues: RwLock::new(HashMap::new()),
            exception_status: RwLock::new(0),
            server_id: RwLock::new(b"rusty-modbus\xFF".to_vec()),
        })
    }

    /// Direct write to an input register (for application-level updates).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AddressOutOfRange`] if `address` is outside the
    /// configured input-register table.
    pub fn set_input_register(&self, address: u16, value: u16) -> Result<(), StoreError> {
        let mut regs = self.input_registers.write();
        let index = check_setup_address("input_registers", address, regs.len())?;
        regs[index] = value;
        Ok(())
    }

    /// Direct write to a discrete input (for application-level updates).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AddressOutOfRange`] if `address` is outside the
    /// configured discrete-input table.
    pub fn set_discrete_input(&self, address: u16, value: bool) -> Result<(), StoreError> {
        let mut inputs = self.discrete_inputs.write();
        let index = check_setup_address("discrete_inputs", address, inputs.len())?;
        inputs.set(index, value);
        Ok(())
    }

    /// Direct write to a holding register (for test setup).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AddressOutOfRange`] if `address` is outside the
    /// configured holding-register table.
    pub fn set_holding_register(&self, address: u16, value: u16) -> Result<(), StoreError> {
        let mut regs = self.holding_registers.write();
        let index = check_setup_address("holding_registers", address, regs.len())?;
        regs[index] = value;
        Ok(())
    }

    /// Direct write to a coil (for test setup).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AddressOutOfRange`] if `address` is outside the
    /// configured coil table.
    pub fn set_coil(&self, address: u16, value: bool) -> Result<(), StoreError> {
        let mut coils = self.coils.write();
        let index = check_setup_address("coils", address, coils.len())?;
        coils.set(index, value);
        Ok(())
    }

    /// Seed a single file-record register (for test/app setup). The file and
    /// record grow lazily so sparse records can be set in any order.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the file or record number is outside the
    /// Modbus file-record address space.
    pub fn set_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        value: u16,
    ) -> Result<(), StoreError> {
        check_setup_file_record(file_number, record_number)?;
        let mut files = self.files.write();
        let file = files.entry(file_number).or_default();
        let idx = usize::from(record_number);
        if idx >= file.len() {
            file.resize(idx + 1, 0);
        }
        file[idx] = value;
        Ok(())
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

fn validate_table_size(table: &'static str, count: usize) -> Result<(), StoreError> {
    if count > MAX_TABLE_SIZE {
        return Err(StoreError::TableTooLarge {
            table,
            count,
            max: MAX_TABLE_SIZE,
        });
    }
    Ok(())
}

fn check_setup_address(table: &'static str, address: u16, len: usize) -> Result<usize, StoreError> {
    let index = usize::from(address);
    if index >= len {
        return Err(StoreError::AddressOutOfRange {
            table,
            address,
            len,
        });
    }
    Ok(index)
}

fn check_setup_file_record(file_number: u16, record_number: u16) -> Result<(), StoreError> {
    if file_number < MIN_FILE_NUMBER {
        return Err(StoreError::FileNumberOutOfRange {
            file_number,
            minimum: MIN_FILE_NUMBER,
        });
    }
    if usize::from(record_number) >= RECORD_COUNT {
        return Err(StoreError::FileRecordOutOfRange {
            record_number,
            maximum: MAX_RECORD_NUMBER,
        });
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
        coils.read_bits(address, quantity, buf)
    }

    async fn read_coils_packed(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        let coils = self.coils.read();
        coils.read_packed(address, quantity, out)
    }

    async fn write_coil(&self, address: u16, value: bool) -> Result<(), ExceptionCode> {
        let mut coils = self.coils.write();
        check_range(address, 1, coils.len())?;
        coils.set(usize::from(address), value);
        Ok(())
    }

    async fn write_coils(&self, address: u16, values: &[bool]) -> Result<(), ExceptionCode> {
        let mut coils = self.coils.write();
        coils.write_bits(address, values)
    }

    async fn write_coils_packed(
        &self,
        address: u16,
        quantity: u16,
        packed_values: &[u8],
    ) -> Result<(), ExceptionCode> {
        let quantity = validate_packed_coils(quantity, packed_values)?;
        let mut coils = self.coils.write();
        coils.write_packed(address, quantity, packed_values)
    }

    async fn read_discrete_inputs(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        let inputs = self.discrete_inputs.read();
        inputs.read_bits(address, quantity, buf)
    }

    async fn read_discrete_inputs_packed(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        let inputs = self.discrete_inputs.read();
        inputs.read_packed(address, quantity, out)
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

    async fn read_holding_registers_be(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        let regs = self.holding_registers.read();
        check_range(address, usize::from(quantity), regs.len())?;
        let start = address as usize;
        let qty = quantity as usize;
        pack_registers_be(&regs[start..start + qty], out)?;
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

    async fn write_registers_be(
        &self,
        address: u16,
        quantity: u16,
        value_bytes: &[u8],
    ) -> Result<(), ExceptionCode> {
        let quantity = validate_register_values_be(quantity, value_bytes)?;
        let mut regs = self.holding_registers.write();
        check_range(address, quantity, regs.len())?;
        let start = address as usize;
        for (slot, chunk) in regs[start..start + quantity]
            .iter_mut()
            .zip(value_bytes.chunks_exact(2))
        {
            *slot = u16::from_be_bytes([chunk[0], chunk[1]]);
        }
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

    async fn read_input_registers_be(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        let regs = self.input_registers.read();
        check_range(address, usize::from(quantity), regs.len())?;
        let start = address as usize;
        let qty = quantity as usize;
        pack_registers_be(&regs[start..start + qty], out)?;
        Ok(qty)
    }

    async fn read_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        record_length: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        file_record::validate_range(file_number, record_number, usize::from(record_length))?;
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

    async fn read_file_record_be(
        &self,
        file_number: u16,
        record_number: u16,
        record_length: u16,
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        let len = usize::from(record_length);
        file_record::validate_range(file_number, record_number, len)?;
        if len > MAX_FILE_RECORD_REGISTERS {
            return Err(ExceptionCode::IllegalDataAddress);
        }
        let files = self.files.read();
        let file = files
            .get(&file_number)
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        let start = usize::from(record_number);
        let end = start
            .checked_add(len)
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        if end > file.len() {
            return Err(ExceptionCode::IllegalDataAddress);
        }
        pack_registers_be(&file[start..end], out)?;
        Ok(len)
    }

    async fn write_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        values: &[u16],
    ) -> Result<(), ExceptionCode> {
        file_record::validate_range(file_number, record_number, values.len())?;
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

    async fn write_file_record_be(
        &self,
        file_number: u16,
        record_number: u16,
        record_length: u16,
        value_bytes: &[u8],
    ) -> Result<(), ExceptionCode> {
        let len = usize::from(record_length);
        if value_bytes.len() != len * 2 {
            return Err(ExceptionCode::IllegalDataValue);
        }
        file_record::validate_range(file_number, record_number, len)?;
        let mut files = self.files.write();
        let file = files.entry(file_number).or_default();
        let start = usize::from(record_number);
        let end = start
            .checked_add(len)
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        if end > file.len() {
            file.resize(end, 0);
        }
        for (slot, chunk) in file[start..end].iter_mut().zip(value_bytes.chunks_exact(2)) {
            *slot = u16::from_be_bytes([chunk[0], chunk[1]]);
        }
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

    async fn read_fifo_queue_be(
        &self,
        address: u16,
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        let queues = self.fifo_queues.read();
        let values = queues
            .get(&address)
            .ok_or(ExceptionCode::IllegalDataAddress)?;
        if values.len() > usize::from(rusty_modbus_types::MAX_FIFO_VALUES) {
            return Err(ExceptionCode::IllegalDataValue);
        }
        pack_registers_be(values, out)?;
        Ok(values.len())
    }

    async fn read_exception_status(&self) -> Result<u8, ExceptionCode> {
        Ok(*self.exception_status.read())
    }

    async fn report_server_id(&self) -> Result<Vec<u8>, ExceptionCode> {
        Ok(self.server_id.read().clone())
    }

    async fn append_server_id(&self, out: &mut Vec<u8>) -> Result<usize, ExceptionCode> {
        let server_id = self.server_id.read();
        if server_id.len() > MAX_SERVER_ID_BYTES {
            return Err(ExceptionCode::ServerDeviceFailure);
        }
        out.extend_from_slice(&server_id);
        Ok(server_id.len())
    }

    async fn append_diagnostic_response(
        &self,
        sub_function: DiagnosticSubFunction,
        data: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<Option<usize>, ExceptionCode> {
        match sub_function {
            DiagnosticSubFunction::ReturnQueryData => {
                if data.len() > MAX_DIAGNOSTIC_RESPONSE_DATA_LEN {
                    return Err(ExceptionCode::ServerDeviceFailure);
                }
                out.extend_from_slice(data);
                Ok(Some(data.len()))
            }
            _ => Err(ExceptionCode::IllegalFunction),
        }
    }
}
