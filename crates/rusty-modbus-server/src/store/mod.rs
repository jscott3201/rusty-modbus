//! Data store abstraction for the four Modbus data tables.

pub mod memory;

use std::future::Future;

use rusty_modbus_types::ExceptionCode;

/// Async trait abstracting the four Modbus data tables (Spec V1.1b3 §4.3).
///
/// All methods are async to support database-backed and remote-proxied stores.
/// Return types use `impl Future<...> + Send` to ensure compatibility with
/// `tokio::spawn` in the server runtime.
///
/// Read methods take `&mut [T]` buffers to avoid heap allocation per request.
pub trait DataStore: Send + Sync {
    // ── Coils (read-write bits) ────────────────────────────────────

    /// Read coil statuses into `buf`. Returns number of coils written.
    ///
    /// # Errors
    ///
    /// Returns `IllegalDataAddress` if `address + quantity` exceeds the address space.
    fn read_coils(&self, address: u16, quantity: u16, buf: &mut [bool]) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

    /// Write a single coil.
    fn write_coil(&self, address: u16, value: bool) -> impl Future<Output = Result<(), ExceptionCode>> + Send;

    /// Write multiple coils.
    fn write_coils(&self, address: u16, values: &[bool]) -> impl Future<Output = Result<(), ExceptionCode>> + Send;

    // ── Discrete Inputs (read-only bits) ───────────────────────────

    /// Read discrete input statuses into `buf`.
    fn read_discrete_inputs(&self, address: u16, quantity: u16, buf: &mut [bool]) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

    // ── Holding Registers (read-write words) ───────────────────────

    /// Read holding registers into `buf`. Returns number of registers written.
    fn read_holding_registers(&self, address: u16, quantity: u16, buf: &mut [u16]) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;

    /// Write a single holding register.
    fn write_register(&self, address: u16, value: u16) -> impl Future<Output = Result<(), ExceptionCode>> + Send;

    /// Write multiple holding registers.
    fn write_registers(&self, address: u16, values: &[u16]) -> impl Future<Output = Result<(), ExceptionCode>> + Send;

    // ── Input Registers (read-only words) ──────────────────────────

    /// Read input registers into `buf`.
    fn read_input_registers(&self, address: u16, quantity: u16, buf: &mut [u16]) -> impl Future<Output = Result<usize, ExceptionCode>> + Send;
}
