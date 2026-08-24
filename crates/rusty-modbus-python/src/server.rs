//! Python bindings for the Modbus server and in-memory data store.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use rusty_modbus_server::{
    CommEventLog, DataStore, InMemoryStore as RustInMemoryStore, ModbusServer as RustModbusServer,
    ServerConfig as RustServerConfig, ServerMetrics as RustServerMetrics,
    ShutdownOutcome as RustShutdownOutcome, StoreConfig as RustStoreConfig, StoreError,
};
use rusty_modbus_types::{DiagnosticSubFunction, ExceptionCode, UnitId};
use tokio::runtime::Runtime;

use crate::errors;

/// Server configuration.
#[pyclass(frozen, from_py_object, module = "rusty_modbus")]
#[derive(Debug, Clone)]
pub struct ServerConfig {
    #[pyo3(get)]
    pub listen_addr: String,
    #[pyo3(get)]
    pub unit_id: u8,
    #[pyo3(get)]
    pub max_connections: usize,
    #[pyo3(get)]
    pub max_transactions: u16,
    #[pyo3(get)]
    pub shutdown_timeout_secs: f64,
}

#[pymethods]
impl ServerConfig {
    #[new]
    #[pyo3(signature = (
        listen_addr=String::from("127.0.0.1:0"),
        unit_id=1,
        max_connections=64,
        max_transactions=16,
        shutdown_timeout_secs=10.0,
    ))]
    fn new(
        listen_addr: String,
        unit_id: u8,
        max_connections: usize,
        max_transactions: u16,
        shutdown_timeout_secs: f64,
    ) -> PyResult<Self> {
        if listen_addr.parse::<SocketAddr>().is_err() {
            return Err(PyValueError::new_err("listen_addr must be host:port"));
        }
        if max_connections == 0 {
            return Err(PyValueError::new_err("max_connections must be >= 1"));
        }
        if max_transactions == 0 {
            return Err(PyValueError::new_err("max_transactions must be >= 1"));
        }
        if !shutdown_timeout_secs.is_finite() || shutdown_timeout_secs <= 0.0 {
            return Err(PyValueError::new_err(
                "shutdown_timeout_secs must be finite and positive",
            ));
        }
        Duration::try_from_secs_f64(shutdown_timeout_secs).map_err(|error| {
            PyValueError::new_err(format!("shutdown_timeout_secs is out of range: {error}"))
        })?;
        Ok(Self {
            listen_addr,
            unit_id,
            max_connections,
            max_transactions,
            shutdown_timeout_secs,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "ServerConfig(listen_addr='{}', unit_id={}, max_connections={}, \
             max_transactions={}, shutdown_timeout_secs={})",
            self.listen_addr,
            self.unit_id,
            self.max_connections,
            self.max_transactions,
            self.shutdown_timeout_secs,
        )
    }
}

impl ServerConfig {
    fn to_rust(&self) -> PyResult<RustServerConfig> {
        let listen_addr = self
            .listen_addr
            .parse::<SocketAddr>()
            .map_err(|e| PyValueError::new_err(format!("invalid listen_addr: {e}")))?;

        let shutdown_timeout = Duration::try_from_secs_f64(self.shutdown_timeout_secs)
            .map_err(|error| PyValueError::new_err(format!("invalid shutdown timeout: {error}")))?;

        Ok(RustServerConfig {
            listen_addr,
            unit_id: UnitId(self.unit_id),
            max_connections: self.max_connections,
            max_transactions: self.max_transactions,
            shutdown_timeout,
            ..RustServerConfig::default()
        })
    }
}

/// Immutable server counter snapshot.
#[pyclass(frozen, skip_from_py_object, module = "rusty_modbus")]
#[derive(Debug, Clone, Copy)]
pub struct ServerMetrics {
    #[pyo3(get)]
    pub active_connections: usize,
    #[pyo3(get)]
    pub active_requests: usize,
    #[pyo3(get)]
    pub accepted_connections: usize,
    #[pyo3(get)]
    pub access_denied_connections: usize,
    #[pyo3(get)]
    pub connection_limit_rejections: usize,
    #[pyo3(get)]
    pub accept_errors: usize,
}

impl From<RustServerMetrics> for ServerMetrics {
    fn from(metrics: RustServerMetrics) -> Self {
        Self {
            active_connections: metrics.active_connections,
            active_requests: metrics.active_requests,
            accepted_connections: metrics.accepted_connections,
            access_denied_connections: metrics.access_denied_connections,
            connection_limit_rejections: metrics.connection_limit_rejections,
            accept_errors: metrics.accept_errors,
        }
    }
}

#[pymethods]
impl ServerMetrics {
    fn __repr__(&self) -> String {
        format!(
            "ServerMetrics(active_connections={}, active_requests={}, accepted_connections={}, \
             access_denied_connections={}, connection_limit_rejections={}, accept_errors={})",
            self.active_connections,
            self.active_requests,
            self.accepted_connections,
            self.access_denied_connections,
            self.connection_limit_rejections,
            self.accept_errors,
        )
    }
}

/// Configuration for an in-memory Modbus data store.
#[pyclass(frozen, from_py_object, module = "rusty_modbus")]
#[derive(Debug, Clone)]
pub struct StoreConfig {
    #[pyo3(get)]
    pub coil_count: usize,
    #[pyo3(get)]
    pub discrete_input_count: usize,
    #[pyo3(get)]
    pub holding_register_count: usize,
    #[pyo3(get)]
    pub input_register_count: usize,
}

#[pymethods]
impl StoreConfig {
    #[new]
    #[pyo3(signature = (
        coil_count=65536,
        discrete_input_count=65536,
        holding_register_count=65536,
        input_register_count=65536,
    ))]
    fn new(
        coil_count: usize,
        discrete_input_count: usize,
        holding_register_count: usize,
        input_register_count: usize,
    ) -> PyResult<Self> {
        let config = RustStoreConfig {
            coil_count,
            discrete_input_count,
            holding_register_count,
            input_register_count,
        };
        config.validate().map_err(store_error_to_pyerr)?;
        Ok(Self {
            coil_count,
            discrete_input_count,
            holding_register_count,
            input_register_count,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "StoreConfig(coil_count={}, discrete_input_count={}, \
             holding_register_count={}, input_register_count={})",
            self.coil_count,
            self.discrete_input_count,
            self.holding_register_count,
            self.input_register_count,
        )
    }
}

impl StoreConfig {
    fn to_rust(&self) -> RustStoreConfig {
        RustStoreConfig {
            coil_count: self.coil_count,
            discrete_input_count: self.discrete_input_count,
            holding_register_count: self.holding_register_count,
            input_register_count: self.input_register_count,
        }
    }
}

/// Thread-safe in-memory Modbus data store.
#[pyclass(module = "rusty_modbus")]
pub struct InMemoryStore {
    inner: Arc<RustInMemoryStore>,
}

#[pymethods]
impl InMemoryStore {
    #[new]
    #[pyo3(signature = (config=None))]
    fn new(config: Option<StoreConfig>) -> PyResult<Self> {
        let config = config.map_or_else(RustStoreConfig::default, |c| c.to_rust());
        let inner = RustInMemoryStore::try_new(config).map_err(store_error_to_pyerr)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Set a coil value.
    #[pyo3(signature = (address, value))]
    fn set_coil(&self, address: u16, value: bool) -> PyResult<()> {
        self.inner
            .set_coil(address, value)
            .map_err(store_error_to_pyerr)
    }

    /// Set a discrete-input value.
    #[pyo3(signature = (address, value))]
    fn set_discrete_input(&self, address: u16, value: bool) -> PyResult<()> {
        self.inner
            .set_discrete_input(address, value)
            .map_err(store_error_to_pyerr)
    }

    /// Set a holding-register value.
    #[pyo3(signature = (address, value))]
    fn set_holding_register(&self, address: u16, value: u16) -> PyResult<()> {
        self.inner
            .set_holding_register(address, value)
            .map_err(store_error_to_pyerr)
    }

    /// Set an input-register value.
    #[pyo3(signature = (address, value))]
    fn set_input_register(&self, address: u16, value: u16) -> PyResult<()> {
        self.inner
            .set_input_register(address, value)
            .map_err(store_error_to_pyerr)
    }

    /// Set one file-record register.
    #[pyo3(signature = (file_number, record_number, value))]
    fn set_file_record(&self, file_number: u16, record_number: u16, value: u16) -> PyResult<()> {
        self.inner
            .set_file_record(file_number, record_number, value)
            .map_err(store_error_to_pyerr)
    }

    /// Set the FIFO queue at `address`.
    #[pyo3(signature = (address, values))]
    fn set_fifo_queue(&self, address: u16, values: Vec<u16>) {
        self.inner.set_fifo_queue(address, values);
    }

    /// Set the exception-status byte.
    #[pyo3(signature = (status))]
    fn set_exception_status(&self, status: u8) {
        self.inner.set_exception_status(status);
    }

    /// Set the report-server-id response payload.
    #[pyo3(signature = (data))]
    fn set_server_id(&self, data: Vec<u8>) {
        self.inner.set_server_id(data);
    }

    fn __repr__(&self) -> String {
        String::from("InMemoryStore()")
    }
}

struct PyDataStore {
    obj: Py<PyAny>,
}

impl PyDataStore {
    fn new(obj: Py<PyAny>) -> Self {
        Self { obj }
    }
}

impl DataStore for PyDataStore {
    async fn read_coils(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        let values = Python::attach(|py| {
            self.obj
                .bind(py)
                .call_method1("read_coils", (address, quantity))
                .and_then(|result| result.extract::<Vec<bool>>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })?;
        copy_returned_values(values, quantity, buf)
    }

    async fn write_coil(&self, address: u16, value: bool) -> Result<(), ExceptionCode> {
        Python::attach(|py| {
            self.obj
                .bind(py)
                .call_method1("write_coil", (address, value))
                .map(|_| ())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn write_coils(&self, address: u16, values: &[bool]) -> Result<(), ExceptionCode> {
        let values = values.to_vec();
        Python::attach(|py| {
            self.obj
                .bind(py)
                .call_method1("write_coils", (address, values))
                .map(|_| ())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn read_discrete_inputs(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        let values = Python::attach(|py| {
            self.obj
                .bind(py)
                .call_method1("read_discrete_inputs", (address, quantity))
                .and_then(|result| result.extract::<Vec<bool>>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })?;
        copy_returned_values(values, quantity, buf)
    }

    async fn read_holding_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let values = Python::attach(|py| {
            self.obj
                .bind(py)
                .call_method1("read_holding_registers", (address, quantity))
                .and_then(|result| result.extract::<Vec<u16>>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })?;
        copy_returned_values(values, quantity, buf)
    }

    async fn write_register(&self, address: u16, value: u16) -> Result<(), ExceptionCode> {
        Python::attach(|py| {
            self.obj
                .bind(py)
                .call_method1("write_register", (address, value))
                .map(|_| ())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn write_registers(&self, address: u16, values: &[u16]) -> Result<(), ExceptionCode> {
        let values = values.to_vec();
        Python::attach(|py| {
            self.obj
                .bind(py)
                .call_method1("write_registers", (address, values))
                .map(|_| ())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn read_input_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let values = Python::attach(|py| {
            self.obj
                .bind(py)
                .call_method1("read_input_registers", (address, quantity))
                .and_then(|result| result.extract::<Vec<u16>>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })?;
        copy_returned_values(values, quantity, buf)
    }

    async fn read_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        record_length: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        let values = Python::attach(|py| {
            let obj = self.obj.bind(py);
            if !has_method(obj, "read_file_record")? {
                return Err(ExceptionCode::IllegalFunction);
            }
            obj.call_method1(
                "read_file_record",
                (file_number, record_number, record_length),
            )
            .and_then(|result| result.extract::<Vec<u16>>())
            .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })?;
        copy_returned_values(values, record_length, buf)
    }

    async fn write_file_record(
        &self,
        file_number: u16,
        record_number: u16,
        values: &[u16],
    ) -> Result<(), ExceptionCode> {
        let values = values.to_vec();
        Python::attach(|py| {
            let obj = self.obj.bind(py);
            if !has_method(obj, "write_file_record")? {
                return Err(ExceptionCode::IllegalFunction);
            }
            obj.call_method1("write_file_record", (file_number, record_number, values))
                .map(|_| ())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn read_fifo_queue(&self, address: u16) -> Result<Vec<u16>, ExceptionCode> {
        Python::attach(|py| {
            let obj = self.obj.bind(py);
            if !has_method(obj, "read_fifo_queue")? {
                return Err(ExceptionCode::IllegalDataAddress);
            }
            obj.call_method1("read_fifo_queue", (address,))
                .and_then(|result| result.extract::<Vec<u16>>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn read_exception_status(&self) -> Result<u8, ExceptionCode> {
        Python::attach(|py| {
            let obj = self.obj.bind(py);
            if !has_method(obj, "read_exception_status")? {
                return Err(ExceptionCode::IllegalFunction);
            }
            obj.call_method0("read_exception_status")
                .and_then(|result| result.extract::<u8>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn get_comm_event_counter(&self) -> Result<(u16, u16), ExceptionCode> {
        Python::attach(|py| {
            let obj = self.obj.bind(py);
            if !has_method(obj, "get_comm_event_counter")? {
                return Err(ExceptionCode::IllegalFunction);
            }
            obj.call_method0("get_comm_event_counter")
                .and_then(|result| result.extract::<(u16, u16)>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn get_comm_event_log(&self) -> Result<CommEventLog, ExceptionCode> {
        Python::attach(|py| {
            let obj = self.obj.bind(py);
            if !has_method(obj, "get_comm_event_log")? {
                return Err(ExceptionCode::IllegalFunction);
            }
            obj.call_method0("get_comm_event_log")
                .and_then(|result| result.extract::<(u16, u16, u16, Vec<u8>)>())
                .map(
                    |(status, event_count, message_count, events)| CommEventLog {
                        status,
                        event_count,
                        message_count,
                        events,
                    },
                )
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn report_server_id(&self) -> Result<Vec<u8>, ExceptionCode> {
        Python::attach(|py| {
            let obj = self.obj.bind(py);
            if !has_method(obj, "report_server_id")? {
                return Err(ExceptionCode::IllegalFunction);
            }
            obj.call_method0("report_server_id")
                .and_then(|result| result.extract::<Vec<u8>>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }

    async fn diagnostic(
        &self,
        sub_function: DiagnosticSubFunction,
        data: &[u8],
    ) -> Result<Option<Vec<u8>>, ExceptionCode> {
        let data = data.to_vec();
        Python::attach(|py| {
            let obj = self.obj.bind(py);
            if !has_method(obj, "diagnostic")? {
                return match sub_function {
                    DiagnosticSubFunction::ReturnQueryData => Ok(Some(data)),
                    _ => Err(ExceptionCode::IllegalFunction),
                };
            }
            obj.call_method1("diagnostic", (sub_function.code(), data))
                .and_then(|result| result.extract::<Option<Vec<u8>>>())
                .map_err(|err| errors::pyerr_to_exception_code(py, err))
        })
    }
}

enum ServerInner {
    Memory(RustModbusServer<RustInMemoryStore>),
    Python(RustModbusServer<PyDataStore>),
}

/// Running Modbus/TCP server.
#[pyclass(module = "rusty_modbus")]
pub struct ModbusServer {
    inner: ServerInner,
    runtime: Runtime,
}

#[pymethods]
impl ModbusServer {
    /// Start a Modbus/TCP server on a background Tokio runtime.
    #[staticmethod]
    #[pyo3(signature = (config=None, store=None))]
    fn start(
        py: Python<'_>,
        config: Option<ServerConfig>,
        store: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let config = config.unwrap_or_else(default_server_config);
        let config = config.to_rust()?;
        let runtime =
            Runtime::new().map_err(|e| PyRuntimeError::new_err(format!("runtime error: {e}")))?;

        if let Some(store) = store {
            if let Some(memory_store) = extract_memory_store(py, &store) {
                let server = runtime
                    .block_on(RustModbusServer::start(config, memory_store))
                    .map_err(errors::server_error_to_pyerr)?;
                Ok(Self {
                    inner: ServerInner::Memory(server),
                    runtime,
                })
            } else {
                let data_store = Arc::new(PyDataStore::new(store));
                let server = runtime
                    .block_on(RustModbusServer::start(config, data_store))
                    .map_err(errors::server_error_to_pyerr)?;
                Ok(Self {
                    inner: ServerInner::Python(server),
                    runtime,
                })
            }
        } else {
            let memory_store = Arc::new(
                RustInMemoryStore::try_new(RustStoreConfig::default())
                    .map_err(store_error_to_pyerr)?,
            );
            let server = runtime
                .block_on(RustModbusServer::start(config, memory_store))
                .map_err(errors::server_error_to_pyerr)?;
            Ok(Self {
                inner: ServerInner::Memory(server),
                runtime,
            })
        }
    }

    /// Local address the server is bound to.
    #[getter]
    fn local_addr(&self) -> String {
        self.local_socket_addr().to_string()
    }

    /// Drain admitted requests or force cancellation at the configured deadline.
    fn stop(&self, py: Python<'_>) -> &'static str {
        let outcome = py.detach(|| match &self.inner {
            ServerInner::Memory(server) => self.runtime.block_on(server.stop()),
            ServerInner::Python(server) => self.runtime.block_on(server.stop()),
        });
        shutdown_outcome_name(outcome)
    }

    /// Return an immutable snapshot of server counters.
    fn metrics(&self) -> ServerMetrics {
        match &self.inner {
            ServerInner::Memory(server) => server.metrics().into(),
            ServerInner::Python(server) => server.metrics().into(),
        }
    }

    /// Sync context manager — enter.
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// Sync context manager — exit (calls stop).
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: &Bound<'_, PyAny>,
        _exc_val: &Bound<'_, PyAny>,
        _exc_tb: &Bound<'_, PyAny>,
    ) -> bool {
        self.stop(py);
        false
    }

    fn __repr__(&self) -> String {
        format!("ModbusServer(local_addr='{}')", self.local_addr())
    }
}

fn shutdown_outcome_name(outcome: RustShutdownOutcome) -> &'static str {
    match outcome {
        RustShutdownOutcome::Drained => "drained",
        RustShutdownOutcome::Forced => "forced",
    }
}

impl ModbusServer {
    fn local_socket_addr(&self) -> SocketAddr {
        match &self.inner {
            ServerInner::Memory(server) => server.local_addr(),
            ServerInner::Python(server) => server.local_addr(),
        }
    }
}

/// Start the historical test helper using the real server and in-memory store.
pub fn start_test_server() -> PyResult<String> {
    let runtime =
        Runtime::new().map_err(|e| PyRuntimeError::new_err(format!("runtime error: {e}")))?;
    let store = Arc::new(seed_test_store());
    let config = RustServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        ..RustServerConfig::default()
    };
    let server = runtime
        .block_on(RustModbusServer::start(config, Arc::clone(&store)))
        .map_err(errors::server_error_to_pyerr)?;
    let addr = server.local_addr().to_string();

    Box::leak(Box::new((server, runtime, store)));
    Ok(addr)
}

fn default_server_config() -> ServerConfig {
    ServerConfig {
        listen_addr: String::from("127.0.0.1:0"),
        unit_id: 1,
        max_connections: 64,
        max_transactions: 16,
        shutdown_timeout_secs: 10.0,
    }
}

fn extract_memory_store(py: Python<'_>, obj: &Py<PyAny>) -> Option<Arc<RustInMemoryStore>> {
    obj.bind(py)
        .extract::<PyRef<'_, InMemoryStore>>()
        .ok()
        .map(|store| Arc::clone(&store.inner))
}

fn seed_test_store() -> RustInMemoryStore {
    let store = RustInMemoryStore::new(RustStoreConfig::default());
    store.set_holding_register(0, 1).unwrap();
    store.set_holding_register(1, 2).unwrap();
    store.set_input_register(0, 1).unwrap();
    store.set_input_register(1, 2).unwrap();
    store.set_coil(0, true).unwrap();
    store.set_coil(1, false).unwrap();
    store.set_coil(2, true).unwrap();
    store.set_discrete_input(0, true).unwrap();
    store.set_discrete_input(1, false).unwrap();
    store.set_discrete_input(2, true).unwrap();
    store.set_fifo_queue(0, vec![10, 11]);
    store
}

fn store_error_to_pyerr(err: StoreError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

fn has_method(obj: &Bound<'_, PyAny>, method: &str) -> Result<bool, ExceptionCode> {
    obj.hasattr(method)
        .map_err(|err| errors::pyerr_to_exception_code(obj.py(), err))
}

fn copy_returned_values<T: Copy>(
    values: Vec<T>,
    quantity: u16,
    buf: &mut [T],
) -> Result<usize, ExceptionCode> {
    let count = values.len();
    if count > usize::from(quantity) || count > buf.len() {
        return Err(ExceptionCode::ServerDeviceFailure);
    }
    buf[..count].copy_from_slice(&values);
    Ok(count)
}
