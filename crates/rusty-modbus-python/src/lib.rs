//! Python bindings for the rusty-modbus client.

use pyo3::prelude::*;

mod client;
mod config;
mod errors;
mod server;
mod sync_client;
mod types;

const PUBLIC_NAMES: &[&str] = &[
    "ModbusError",
    "TimeoutError",
    "ConnectionError",
    "ModbusExceptionError",
    "RetryError",
    "IllegalFunctionError",
    "IllegalDataAddressError",
    "IllegalDataValueError",
    "ServerDeviceFailureError",
    "AcknowledgeError",
    "ServerDeviceBusyError",
    "NegativeAcknowledgeError",
    "MemoryParityError",
    "GatewayPathUnavailableError",
    "GatewayTargetDeviceFailedToRespondError",
    "ClientConfig",
    "TlsConfig",
    "RetryConfig",
    "DeviceIdentification",
    "ModbusClient",
    "SyncModbusClient",
    "ServerConfig",
    "StoreConfig",
    "InMemoryStore",
    "ModbusServer",
];

/// Start an embedded Modbus test server on a random port (synchronous).
///
/// Returns the ``host:port`` address string. The server runs on a dedicated
/// background tokio runtime that lives for the duration of the process.
#[pyfunction]
fn _start_test_server() -> PyResult<String> {
    server::start_test_server()
}

/// The `rusty_modbus` Python module.
#[pymodule(gil_used = false)]
fn rusty_modbus(m: &Bound<'_, PyModule>) -> PyResult<()> {
    errors::register(m)?;
    m.add_class::<config::ClientConfig>()?;
    m.add_class::<config::TlsConfig>()?;
    m.add_class::<config::RetryConfig>()?;
    m.add_class::<types::DeviceIdentification>()?;
    m.add_class::<client::ModbusClient>()?;
    m.add_class::<sync_client::SyncModbusClient>()?;
    m.add_class::<server::ServerConfig>()?;
    m.add_class::<server::StoreConfig>()?;
    m.add_class::<server::InMemoryStore>()?;
    m.add_class::<server::ModbusServer>()?;
    m.add_function(wrap_pyfunction!(_start_test_server, m)?)?;
    m.add("__all__", PUBLIC_NAMES)?;
    Ok(())
}
