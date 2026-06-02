//! Data store abstraction for the four Modbus data tables, plus optional
//! file-record, FIFO-queue, and serial-line-diagnostic capabilities.

pub mod memory;

use std::future::Future;

use rusty_modbus_types::{DiagnosticSubFunction, ExceptionCode};

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

    // ── Discrete Inputs (read-only bits) ───────────────────────────

    /// Read discrete input statuses into `buf`.
    fn read_discrete_inputs(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

    // ── Holding Registers (read-write words) ───────────────────────

    /// Read holding registers into `buf`. Returns number of registers written.
    fn read_holding_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

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

    // ── Input Registers (read-only words) ──────────────────────────

    /// Read input registers into `buf`.
    fn read_input_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

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

    /// Report a device-specific server-identification blob (FC 0x11, §6.13).
    ///
    /// The returned bytes become the response's `data` field (the handler
    /// prepends the byte count). The default returns
    /// [`ExceptionCode::IllegalFunction`].
    fn report_server_id(&self) -> impl Future<Output = Result<Vec<u8>, ExceptionCode>> + Send {
        async { Err(ExceptionCode::IllegalFunction) }
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
}
