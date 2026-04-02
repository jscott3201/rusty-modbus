//! Python bindings for the rusty-modbus client.

use pyo3::prelude::*;

mod config;
mod errors;
mod types;

/// The `rusty_modbus` Python module.
#[pymodule]
fn rusty_modbus(m: &Bound<'_, PyModule>) -> PyResult<()> {
    errors::register(m)?;
    m.add_class::<config::ClientConfig>()?;
    m.add_class::<config::TlsConfig>()?;
    m.add_class::<config::RetryConfig>()?;
    m.add_class::<types::DeviceIdentification>()?;
    Ok(())
}
