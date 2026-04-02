//! Python configuration classes (stub).
use pyo3::prelude::*;
#[pyclass(frozen, module = "rusty_modbus")] pub struct ClientConfig;
#[pyclass(frozen, module = "rusty_modbus")] pub struct TlsConfig;
#[pyclass(frozen, module = "rusty_modbus")] pub struct RetryConfig;
