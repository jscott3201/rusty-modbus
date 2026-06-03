//! Data store abstraction for the four Modbus data tables, plus optional
//! file-record, FIFO-queue, and serial-line-diagnostic capabilities.

pub mod memory;

use std::future::Future;

use rusty_modbus_types::{
    DiagnosticSubFunction, ExceptionCode, MAX_FIFO_VALUES, MAX_PDU_SIZE, MAX_READ_COILS,
    MAX_READ_DISCRETE_INPUTS, MAX_READ_REGISTERS, MAX_WRITE_COILS, MAX_WRITE_REGISTERS,
};

pub(crate) const MAX_FILE_RECORD_REGISTERS: usize = 122;
pub(crate) const MAX_COMM_EVENT_LOG_EVENTS: usize = 64;
pub(crate) const MAX_DIAGNOSTIC_RESPONSE_DATA_LEN: usize = MAX_PDU_SIZE - 3;
pub(crate) const MAX_SERVER_ID_BYTES: usize = MAX_PDU_SIZE - 2;

/// Fixed fields returned with a Get Comm Event Log response (FC 0x0C).
///
/// Event bytes are carried separately by append-style datastore hooks so
/// direct-access stores can write them into the final response buffer.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommEventLogMeta {
    /// Status word: `0x0000` ready, `0xFFFF` busy.
    pub status: u16,
    /// Event counter value.
    pub event_count: u16,
    /// Message counter value.
    pub message_count: u16,
}

/// Snapshot of the communications event log returned by Get Comm Event Log
/// (FC 0x0C, Spec V1.1b3 §6.10).
///
/// Owned (rather than borrowing the codec response type) so a [`DataStore`] can
/// build it behind its async boundary; the handler derives the wire `byte_count`
/// as `events.len() + 6`.
#[derive(Debug, Clone, Default)]
pub struct CommEventLog {
    /// Status word: `0x0000` ready, `0xFFFF` busy.
    pub status: u16,
    /// Event counter value.
    pub event_count: u16,
    /// Message counter value.
    pub message_count: u16,
    /// Event bytes (0..=64).
    pub events: Vec<u8>,
}

pub(crate) fn unpack_packed_coils(
    quantity: u16,
    packed_values: &[u8],
    out: &mut [bool],
) -> Result<usize, ExceptionCode> {
    let quantity = validate_packed_coils(quantity, packed_values)?;
    if out.len() < quantity {
        return Err(ExceptionCode::IllegalDataValue);
    }
    for (byte_index, &byte) in packed_values.iter().enumerate() {
        let start = byte_index * 8;
        let bit_count = (quantity - start).min(8);
        for bit in 0..bit_count {
            out[start + bit] = (byte >> bit) & 1 == 1;
        }
    }
    Ok(quantity)
}

pub(crate) fn unpack_register_values_be(
    quantity: u16,
    value_bytes: &[u8],
    out: &mut [u16],
) -> Result<usize, ExceptionCode> {
    let quantity = validate_register_values_be(quantity, value_bytes)?;
    if out.len() < quantity {
        return Err(ExceptionCode::IllegalDataValue);
    }
    for (slot, chunk) in out
        .iter_mut()
        .zip(value_bytes.chunks_exact(2))
        .take(quantity)
    {
        *slot = u16::from_be_bytes([chunk[0], chunk[1]]);
    }
    Ok(quantity)
}

pub(crate) fn validate_packed_coils(
    quantity: u16,
    packed_values: &[u8],
) -> Result<usize, ExceptionCode> {
    if quantity == 0 || quantity > MAX_WRITE_COILS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let expected = usize::from(quantity).div_ceil(8);
    if packed_values.len() != expected {
        return Err(ExceptionCode::IllegalDataValue);
    }
    Ok(usize::from(quantity))
}

pub(crate) fn validate_register_values_be(
    quantity: u16,
    value_bytes: &[u8],
) -> Result<usize, ExceptionCode> {
    if quantity == 0 || quantity > MAX_WRITE_REGISTERS {
        return Err(ExceptionCode::IllegalDataValue);
    }
    let expected = usize::from(quantity) * 2;
    if value_bytes.len() != expected {
        return Err(ExceptionCode::IllegalDataValue);
    }
    Ok(usize::from(quantity))
}

pub(crate) fn pack_coils(bits: &[bool], out: &mut [u8]) -> Result<(), ExceptionCode> {
    let byte_count = bits.len().div_ceil(8);
    if out.len() < byte_count {
        return Err(ExceptionCode::IllegalDataValue);
    }
    for (byte_index, out_byte) in out[..byte_count].iter_mut().enumerate() {
        let start = byte_index * 8;
        let end = (start + 8).min(bits.len());
        let mut byte = 0u8;
        for (bit, &value) in bits[start..end].iter().enumerate() {
            byte |= u8::from(value) << bit;
        }
        *out_byte = byte;
    }
    Ok(())
}

pub(crate) fn pack_registers_be(registers: &[u16], out: &mut [u8]) -> Result<(), ExceptionCode> {
    let byte_count = registers.len() * 2;
    if out.len() < byte_count {
        return Err(ExceptionCode::IllegalDataValue);
    }
    for (chunk, &value) in out[..byte_count].chunks_exact_mut(2).zip(registers) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    Ok(())
}

/// Async trait abstracting the four Modbus data tables (Spec V1.1b3 §4.3).
///
/// All methods are async to support database-backed and remote-proxied stores.
/// Return types use `impl Future<...> + Send` to ensure compatibility with
/// `tokio::spawn` in the server runtime.
///
/// Read methods take `&mut [T]` buffers to avoid heap allocation per request.
///
/// The eight methods covering the four core data tables (coils, discrete inputs,
/// holding/input registers) are **required**. The remaining methods — file
/// records, FIFO queues, and the serial-line diagnostics family — are
/// **optional**: each has a default body returning the spec-correct exception
/// for an unsupported capability, so existing implementations keep compiling and
/// only override the capabilities they actually serve.
pub trait DataStore: Send + Sync {
    // ── Coils (read-write bits) ────────────────────────────────────

    /// Read coil statuses into `buf`. Returns number of coils written.
    ///
    /// # Errors
    ///
    /// Returns `IllegalDataAddress` if `address + quantity` exceeds the address space.
    fn read_coils(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

    /// Read coil statuses directly into the Modbus packed-bit wire format.
    ///
    /// The default implementation delegates to [`Self::read_coils`] through a
    /// bounded scratch buffer. Stores with direct table access can override this
    /// method to avoid the intermediate bool slice and response repacking.
    fn read_coils_packed(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        async move {
            let mut values = [false; MAX_READ_COILS as usize];
            let count = self.read_coils(address, quantity, &mut values).await?;
            if count > values.len() || count > usize::from(quantity) {
                return Err(ExceptionCode::ServerDeviceFailure);
            }
            pack_coils(&values[..count], out)?;
            Ok(count)
        }
    }

    /// Write a single coil.
    fn write_coil(
        &self,
        address: u16,
        value: bool,
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send;

    /// Write multiple coils.
    fn write_coils(
        &self,
        address: u16,
        values: &[bool],
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send;

    /// Write multiple coils from the Modbus packed-bit wire representation.
    ///
    /// The default implementation unpacks into a bounded stack buffer and then
    /// delegates to [`Self::write_coils`]. Stores with direct table access can
    /// override this method to avoid the intermediate bool slice entirely.
    fn write_coils_packed(
        &self,
        address: u16,
        quantity: u16,
        packed_values: &[u8],
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send {
        async move {
            let mut values = [false; MAX_WRITE_COILS as usize];
            let quantity = unpack_packed_coils(quantity, packed_values, &mut values)?;
            self.write_coils(address, &values[..quantity]).await
        }
    }

    // ── Discrete Inputs (read-only bits) ───────────────────────────

    /// Read discrete input statuses into `buf`.
    fn read_discrete_inputs(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

    /// Read discrete input statuses directly into the Modbus packed-bit wire format.
    ///
    /// The default implementation delegates to [`Self::read_discrete_inputs`]
    /// through a bounded scratch buffer. Stores with direct table access can
    /// override this method to avoid the intermediate bool slice.
    fn read_discrete_inputs_packed(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        async move {
            let mut values = [false; MAX_READ_DISCRETE_INPUTS as usize];
            let count = self
                .read_discrete_inputs(address, quantity, &mut values)
                .await?;
            if count > values.len() || count > usize::from(quantity) {
                return Err(ExceptionCode::ServerDeviceFailure);
            }
            pack_coils(&values[..count], out)?;
            Ok(count)
        }
    }

    // ── Holding Registers (read-write words) ───────────────────────

    /// Read holding registers into `buf`. Returns number of registers written.
    fn read_holding_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

    /// Read holding registers directly into big-endian Modbus wire bytes.
    ///
    /// The default implementation delegates to [`Self::read_holding_registers`]
    /// through a bounded scratch buffer. Stores with direct table access can
    /// override this method to avoid the intermediate register slice and
    /// response encoding pass.
    fn read_holding_registers_be(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        async move {
            let mut values = [0u16; MAX_READ_REGISTERS as usize];
            let count = self
                .read_holding_registers(address, quantity, &mut values)
                .await?;
            if count > values.len() || count > usize::from(quantity) {
                return Err(ExceptionCode::ServerDeviceFailure);
            }
            pack_registers_be(&values[..count], out)?;
            Ok(count)
        }
    }

    /// Write a single holding register.
    fn write_register(
        &self,
        address: u16,
        value: u16,
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send;

    /// Write multiple holding registers.
    fn write_registers(
        &self,
        address: u16,
        values: &[u16],
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send;

    /// Write multiple holding registers from big-endian Modbus wire bytes.
    ///
    /// The default implementation unpacks into a bounded stack buffer and then
    /// delegates to [`Self::write_registers`]. Stores with direct table access
    /// can override this method to avoid the intermediate register slice.
    fn write_registers_be(
        &self,
        address: u16,
        quantity: u16,
        value_bytes: &[u8],
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send {
        async move {
            let mut values = [0u16; MAX_WRITE_REGISTERS as usize];
            let quantity = unpack_register_values_be(quantity, value_bytes, &mut values)?;
            self.write_registers(address, &values[..quantity]).await
        }
    }

    // ── Input Registers (read-only words) ──────────────────────────

    /// Read input registers into `buf`.
    fn read_input_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

    /// Read input registers directly into big-endian Modbus wire bytes.
    ///
    /// The default implementation delegates to [`Self::read_input_registers`]
    /// through a bounded scratch buffer. Stores with direct table access can
    /// override this method to avoid the intermediate register slice.
    fn read_input_registers_be(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u8],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        async move {
            let mut values = [0u16; MAX_READ_REGISTERS as usize];
            let count = self
                .read_input_registers(address, quantity, &mut values)
                .await?;
            if count > values.len() || count > usize::from(quantity) {
                return Err(ExceptionCode::ServerDeviceFailure);
            }
            pack_registers_be(&values[..count], out)?;
            Ok(count)
        }
    }

    // ── File Records (FC 0x14 / 0x15) — optional capability ────────

    /// Read one file sub-record (`record_length` registers from `record_number`
    /// in file `file_number`) into `buf`; returns the number of registers
    /// written (Spec V1.1b3 §6.14).
    ///
    /// The default returns [`ExceptionCode::IllegalFunction`] — a store that
    /// does not maintain file records reports `0x01` for FC 0x14.
    fn read_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        record_length: u16,
        buf: &mut [u16],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        async move {
            let _ = (file_number, record_number, record_length, buf);
            Err(ExceptionCode::IllegalFunction)
        }
    }

    /// Read one file sub-record directly into big-endian Modbus wire bytes.
    ///
    /// The default implementation delegates to [`Self::read_file_record`] with
    /// a bounded scratch buffer and then packs that scratch into `out`. Stores
    /// with direct file access can override this method to avoid the
    /// intermediate register slice.
    fn read_file_record_be(
        &self,
        file_number: u16,
        record_number: u16,
        record_length: u16,
        out: &mut [u8],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        async move {
            let mut values = [0u16; MAX_FILE_RECORD_REGISTERS];
            let count = self
                .read_file_record(file_number, record_number, record_length, &mut values)
                .await?;
            if count > values.len() || count > usize::from(record_length) {
                return Err(ExceptionCode::ServerDeviceFailure);
            }
            pack_registers_be(&values[..count], out)?;
            Ok(count)
        }
    }

    /// Write `values` to `record_number` in file `file_number` (Spec V1.1b3 §6.15).
    ///
    /// The default returns [`ExceptionCode::IllegalFunction`].
    fn write_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        values: &[u16],
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send {
        async move {
            let _ = (file_number, record_number, values);
            Err(ExceptionCode::IllegalFunction)
        }
    }

    /// Write one file sub-record from big-endian Modbus wire bytes.
    ///
    /// The default implementation unpacks the wire bytes and delegates to
    /// [`Self::write_file_record`]. Stores with direct file access can override
    /// this method to avoid the intermediate register vector.
    fn write_file_record_be(
        &self,
        file_number: u16,
        record_number: u16,
        record_length: u16,
        value_bytes: &[u8],
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send {
        async move {
            let expected = usize::from(record_length) * 2;
            if value_bytes.len() != expected {
                return Err(ExceptionCode::IllegalDataValue);
            }
            let values: Vec<u16> = value_bytes
                .chunks_exact(2)
                .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
                .collect();
            self.write_file_record(file_number, record_number, &values)
                .await
        }
    }

    // ── FIFO Queue (FC 0x18) — optional capability ─────────────────

    /// Return a non-destructive snapshot of the FIFO queue whose pointer is at
    /// `address` (at most 31 values; Spec V1.1b3 §6.18 — reading the queue MUST
    /// NOT drain it).
    ///
    /// The default returns [`ExceptionCode::IllegalDataAddress`] — there is no
    /// FIFO at the requested address (Figure 28).
    fn read_fifo_queue(
        &self,
        address: u16,
    ) -> impl Future<Output = Result<Vec<u16>, ExceptionCode>> + Send {
        async move {
            let _ = address;
            Err(ExceptionCode::IllegalDataAddress)
        }
    }

    /// Read a FIFO queue snapshot directly into big-endian Modbus wire bytes.
    ///
    /// The default implementation delegates to [`Self::read_fifo_queue`] and
    /// packs the returned values. Stores with direct queue access can override
    /// this method to avoid cloning the queue and allocating an intermediate
    /// byte buffer.
    fn read_fifo_queue_be(
        &self,
        address: u16,
        out: &mut [u8],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        async move {
            let values = self.read_fifo_queue(address).await?;
            if values.len() > usize::from(MAX_FIFO_VALUES) {
                return Err(ExceptionCode::IllegalDataValue);
            }
            pack_registers_be(&values, out)?;
            Ok(values.len())
        }
    }

    // ── Serial-line diagnostics (FC 0x07/0x08/0x0B/0x0C/0x11) ───────

    /// Read the eight device-specific exception-status coils as one byte
    /// (FC 0x07, §6.7). The default returns [`ExceptionCode::IllegalFunction`].
    fn read_exception_status(&self) -> impl Future<Output = Result<u8, ExceptionCode>> + Send {
        async { Err(ExceptionCode::IllegalFunction) }
    }

    /// Get the comm event counter as `(status, event_count)` (FC 0x0B, §6.9).
    /// The default returns [`ExceptionCode::IllegalFunction`].
    fn get_comm_event_counter(
        &self,
    ) -> impl Future<Output = Result<(u16, u16), ExceptionCode>> + Send {
        async { Err(ExceptionCode::IllegalFunction) }
    }

    /// Get the communications event log (FC 0x0C, §6.10).
    /// The default returns [`ExceptionCode::IllegalFunction`].
    fn get_comm_event_log(
        &self,
    ) -> impl Future<Output = Result<CommEventLog, ExceptionCode>> + Send {
        async { Err(ExceptionCode::IllegalFunction) }
    }

    /// Append FC 0x0C communication event bytes to `out`.
    ///
    /// The default delegates to [`Self::get_comm_event_log`]. Direct-access
    /// stores can override this method to avoid materializing the event list in
    /// an intermediate `Vec` before response construction.
    fn append_comm_event_log(
        &self,
        out: &mut Vec<u8>,
    ) -> impl Future<Output = Result<CommEventLogMeta, ExceptionCode>> + Send {
        async move {
            let log = self.get_comm_event_log().await?;
            if log.events.len() > MAX_COMM_EVENT_LOG_EVENTS {
                return Err(ExceptionCode::ServerDeviceFailure);
            }
            out.extend_from_slice(&log.events);
            Ok(CommEventLogMeta {
                status: log.status,
                event_count: log.event_count,
                message_count: log.message_count,
            })
        }
    }

    /// Report a device-specific server-identification blob (FC 0x11, §6.13).
    ///
    /// The returned bytes become the response's `data` field (the handler
    /// prepends the byte count). The default returns
    /// [`ExceptionCode::IllegalFunction`].
    fn report_server_id(&self) -> impl Future<Output = Result<Vec<u8>, ExceptionCode>> + Send {
        async { Err(ExceptionCode::IllegalFunction) }
    }

    /// Append the FC 0x11 server-identification data bytes to `out`.
    ///
    /// The default delegates to [`Self::report_server_id`]. Direct-access
    /// stores can override this method to avoid cloning the identification blob
    /// before the handler copies it into the final response PDU.
    fn append_server_id(
        &self,
        out: &mut Vec<u8>,
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        async move {
            let data = self.report_server_id().await?;
            if data.len() > MAX_SERVER_ID_BYTES {
                return Err(ExceptionCode::ServerDeviceFailure);
            }
            out.extend_from_slice(&data);
            Ok(data.len())
        }
    }

    /// Execute a Diagnostics sub-function (FC 0x08, §6.8).
    ///
    /// `Ok(Some(data))` echoes `data` back in the response; `Ok(None)`
    /// suppresses the response entirely (per the spec, e.g. Force Listen Only
    /// Mode); `Err` returns an exception.
    ///
    /// The default loops back Return Query Data (0x0000) and reports every other
    /// sub-function as [`ExceptionCode::IllegalFunction`] (Figure 18: an
    /// unsupported sub-function is an illegal *function*, not an illegal data
    /// value).
    fn diagnostic(
        &self,
        sub_function: DiagnosticSubFunction,
        data: &[u8],
    ) -> impl Future<Output = Result<Option<Vec<u8>>, ExceptionCode>> + Send {
        async move {
            match sub_function {
                DiagnosticSubFunction::ReturnQueryData => Ok(Some(data.to_vec())),
                _ => Err(ExceptionCode::IllegalFunction),
            }
        }
    }

    /// Append Diagnostics response data bytes to `out`.
    ///
    /// The default delegates to [`Self::diagnostic`]. Stores that can produce a
    /// response from borrowed request bytes can override this method to avoid an
    /// intermediate owned `Vec` before the handler builds the final response PDU.
    fn append_diagnostic_response(
        &self,
        sub_function: DiagnosticSubFunction,
        data: &[u8],
        out: &mut Vec<u8>,
    ) -> impl Future<Output = Result<Option<usize>, ExceptionCode>> + Send {
        async move {
            let Some(data) = self.diagnostic(sub_function, data).await? else {
                return Ok(None);
            };
            if data.len() > MAX_DIAGNOSTIC_RESPONSE_DATA_LEN {
                return Err(ExceptionCode::ServerDeviceFailure);
            }
            out.extend_from_slice(&data);
            Ok(Some(data.len()))
        }
    }
}
