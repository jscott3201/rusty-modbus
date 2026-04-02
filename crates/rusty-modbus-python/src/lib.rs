//! Python bindings for the rusty-modbus client.

use pyo3::prelude::*;

mod client;
mod config;
mod errors;
mod sync_client;
mod types;

/// The `rusty_modbus` Python module.
#[pymodule]
fn rusty_modbus(m: &Bound<'_, PyModule>) -> PyResult<()> {
    errors::register(m)?;
    m.add_class::<config::ClientConfig>()?;
    m.add_class::<config::TlsConfig>()?;
    m.add_class::<config::RetryConfig>()?;
    m.add_class::<types::DeviceIdentification>()?;
    m.add_class::<client::ModbusClient>()?;
    m.add_class::<sync_client::SyncModbusClient>()?;
    Ok(())
}
