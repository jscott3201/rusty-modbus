//! Python configuration classes.

use std::path::PathBuf;
use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rusty_modbus_types::UnitId;

/// Client connection configuration.
#[pyclass(frozen, from_py_object, module = "rusty_modbus")]
#[derive(Debug, Clone)]
pub struct ClientConfig {
    #[pyo3(get)]
    pub unit_id: u8,
    #[pyo3(get)]
    pub timeout_secs: f64,
    #[pyo3(get)]
    pub max_in_flight: usize,
    #[pyo3(get)]
    pub retry: Option<RetryConfig>,
    #[pyo3(get)]
    pub shutdown_timeout_secs: f64,
}

#[pymethods]
impl ClientConfig {
    #[new]
    #[pyo3(signature = (unit_id=255, timeout_secs=5.0, max_in_flight=16, retry=None, shutdown_timeout_secs=10.0))]
    fn new(
        unit_id: u8,
        timeout_secs: f64,
        max_in_flight: usize,
        retry: Option<RetryConfig>,
        shutdown_timeout_secs: f64,
    ) -> PyResult<Self> {
        if !timeout_secs.is_finite() || timeout_secs <= 0.0 {
            return Err(PyValueError::new_err(
                "timeout_secs must be finite and positive",
            ));
        }
        if max_in_flight == 0 {
            return Err(PyValueError::new_err("max_in_flight must be >= 1"));
        }
        if !shutdown_timeout_secs.is_finite() || shutdown_timeout_secs <= 0.0 {
            return Err(PyValueError::new_err(
                "shutdown_timeout_secs must be finite and positive",
            ));
        }
        Ok(Self {
            unit_id,
            timeout_secs,
            max_in_flight,
            retry,
            shutdown_timeout_secs,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "ClientConfig(unit_id={}, timeout_secs={}, max_in_flight={}, retry={:?}, shutdown_timeout_secs={})",
            self.unit_id,
            self.timeout_secs,
            self.max_in_flight,
            self.retry,
            self.shutdown_timeout_secs,
        )
    }
}

impl ClientConfig {
    /// Convert to the Rust `ClientConfig`.
    pub fn to_rust(&self) -> rusty_modbus_client::ClientConfig {
        let retry = self
            .retry
            .as_ref()
            .map(RetryConfig::to_rust)
            .unwrap_or_default();
        rusty_modbus_client::ClientConfig {
            unit_id: UnitId(self.unit_id),
            timeout: Duration::from_secs_f64(self.timeout_secs),
            max_in_flight: self.max_in_flight,
            retry,
            shutdown_timeout: Duration::from_secs_f64(self.shutdown_timeout_secs),
        }
    }
}

/// Retry configuration.
#[pyclass(frozen, from_py_object, module = "rusty_modbus")]
#[derive(Debug, Clone)]
pub struct RetryConfig {
    #[pyo3(get)]
    pub max_retries: u32,
    #[pyo3(get)]
    pub retry_delay_ms: u64,
}

#[pymethods]
impl RetryConfig {
    #[new]
    #[pyo3(signature = (max_retries=3, retry_delay_ms=100))]
    fn new(max_retries: u32, retry_delay_ms: u64) -> Self {
        Self {
            max_retries,
            retry_delay_ms,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "RetryConfig(max_retries={}, retry_delay_ms={})",
            self.max_retries, self.retry_delay_ms
        )
    }
}

impl RetryConfig {
    pub fn to_rust(&self) -> rusty_modbus_client::RetryConfig {
        rusty_modbus_client::RetryConfig {
            max_retries: self.max_retries,
            retry_delay: Duration::from_millis(self.retry_delay_ms),
            ..rusty_modbus_client::RetryConfig::default()
        }
    }
}

/// TLS connection configuration.
#[pyclass(frozen, from_py_object, module = "rusty_modbus")]
#[derive(Debug, Clone)]
pub struct TlsConfig {
    #[pyo3(get)]
    pub ca_cert: String,
    #[pyo3(get)]
    pub client_cert: String,
    #[pyo3(get)]
    pub client_key: String,
    #[pyo3(get)]
    pub timeout_secs: f64,
}

#[pymethods]
impl TlsConfig {
    #[new]
    #[pyo3(signature = (ca_cert, client_cert, client_key, timeout_secs=5.0))]
    fn new(
        ca_cert: String,
        client_cert: String,
        client_key: String,
        timeout_secs: f64,
    ) -> PyResult<Self> {
        if timeout_secs <= 0.0 {
            return Err(PyValueError::new_err("timeout_secs must be positive"));
        }
        Ok(Self {
            ca_cert,
            client_cert,
            client_key,
            timeout_secs,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "TlsConfig(ca_cert='{}', client_cert='{}', client_key='...', timeout_secs={})",
            self.ca_cert, self.client_cert, self.timeout_secs,
        )
    }
}

impl TlsConfig {
    pub fn to_rust(&self) -> rusty_modbus_tls::TlsClientConfig {
        let timeout = Duration::from_secs_f64(self.timeout_secs);
        rusty_modbus_tls::TlsClientConfig {
            ca_cert: PathBuf::from(&self.ca_cert),
            client_cert: PathBuf::from(&self.client_cert),
            client_key: PathBuf::from(&self.client_key),
            server_name: None,
            connect_timeout: timeout,
            read_timeout: Some(timeout),
            write_timeout: Some(timeout),
        }
    }
}
