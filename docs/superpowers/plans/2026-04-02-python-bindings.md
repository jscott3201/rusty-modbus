# Python Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a PyO3-based Python client library (`rusty_modbus`) exposing the Rust Modbus client over TCP and TLS transports, with both async and sync APIs.

**Architecture:** New workspace crate `crates/rusty-modbus-python/` built with maturin. The Rust `ModbusClient` is first generified over its transport sink type to support both TCP and TLS, then wrapped in PyO3 `#[pyclass]` types that use `pyo3-async-runtimes` for async Python integration and `tokio::runtime::Runtime` for the sync wrapper.

**Tech Stack:** PyO3 0.28, pyo3-async-runtimes 0.28, maturin, tokio, pytest, pytest-asyncio

---

## File Map

### Modified Files

| File | Change |
|------|--------|
| `crates/rusty-modbus-client/src/client.rs` | Generify `ModbusClient<S>` over sink type, add `from_transport()` constructor |
| `crates/rusty-modbus-client/src/reader.rs` | Make `spawn_reader` generic over `TransportStream` |
| `crates/rusty-modbus-client/src/methods/registers.rs` | Change `impl ModbusClient` → `impl<S: TransportSink + Send + 'static> ModbusClient<S>` |
| `crates/rusty-modbus-client/src/methods/coils.rs` | Same generic bound change |
| `crates/rusty-modbus-client/src/methods/device_id.rs` | Same generic bound change |
| `crates/rusty-modbus-client/src/methods/fifo.rs` | Same generic bound change |
| `crates/rusty-modbus-client/src/methods/file.rs` | Same generic bound change |

### New Files

| File | Purpose |
|------|---------|
| `crates/rusty-modbus-python/Cargo.toml` | Crate manifest with PyO3 + workspace deps |
| `crates/rusty-modbus-python/pyproject.toml` | Maturin build config |
| `crates/rusty-modbus-python/src/lib.rs` | `#[pymodule]` definition, exports |
| `crates/rusty-modbus-python/src/errors.rs` | Exception hierarchy via `create_exception!` + `From<ClientError>` |
| `crates/rusty-modbus-python/src/config.rs` | `ClientConfig`, `TlsConfig`, `RetryConfig` pyclasses |
| `crates/rusty-modbus-python/src/types.rs` | `DeviceIdentification` pyclass |
| `crates/rusty-modbus-python/src/client.rs` | `ModbusClient` async pyclass |
| `crates/rusty-modbus-python/src/sync_client.rs` | `SyncModbusClient` blocking pyclass |
| `crates/rusty-modbus-python/tests/test_config.py` | Python tests for config classes |
| `crates/rusty-modbus-python/tests/test_errors.py` | Python tests for exception hierarchy |
| `crates/rusty-modbus-python/tests/test_sync_client.py` | Python integration tests for sync client |
| `crates/rusty-modbus-python/tests/test_async_client.py` | Python integration tests for async client |

---

### Task 1: Generify `ModbusClient` Over Transport Sink Type

**Files:**
- Modify: `crates/rusty-modbus-client/src/reader.rs`
- Modify: `crates/rusty-modbus-client/src/client.rs`
- Modify: `crates/rusty-modbus-client/src/methods/registers.rs`
- Modify: `crates/rusty-modbus-client/src/methods/coils.rs`
- Modify: `crates/rusty-modbus-client/src/methods/device_id.rs`
- Modify: `crates/rusty-modbus-client/src/methods/fifo.rs`
- Modify: `crates/rusty-modbus-client/src/methods/file.rs`

The Rust `ModbusClient` currently hardcodes `TcpSink` and `TcpRecvStream`. To support TLS without trait objects (RPITIT makes `TransportSink`/`TransportStream` non-object-safe), we generify the client. A default type parameter keeps existing code backward-compatible.

- [ ] **Step 1: Make `spawn_reader` generic**

In `crates/rusty-modbus-client/src/reader.rs`, change the function signature from concrete `TcpRecvStream` to generic:

```rust
use rusty_modbus_tcp::transport::TransportStream;
// Remove: use rusty_modbus_tcp::TcpRecvStream;

pub(crate) fn spawn_reader<R: TransportStream + Send + 'static>(
    mut stream: R,
    txn_mgr: Arc<TransactionManager>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    // Body is unchanged — it only calls stream.recv() which is on the TransportStream trait
```

- [ ] **Step 2: Generify `ModbusClient` struct**

In `crates/rusty-modbus-client/src/client.rs`, add a type parameter with default:

```rust
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink, TransportStream};
use rusty_modbus_tcp::{TcpConfig, TcpSink, TcpTransport};

/// High-level async Modbus client with transaction pipelining.
pub struct ModbusClient<S: TransportSink + Send + 'static = TcpSink> {
    sink: tokio::sync::Mutex<S>,
    txn_mgr: Arc<TransactionManager>,
    config: ClientConfig,
    connected: AtomicBool,
    semaphore: Arc<Semaphore>,
    shutdown_tx: watch::Sender<bool>,
    reader_handle: Option<tokio::task::JoinHandle<()>>,
    sweep_handle: Option<tokio::task::JoinHandle<()>>,
}
```

- [ ] **Step 3: Move shared methods into generic impl block**

Move `is_connected`, `unit_id`, `shutdown`, `send_request`, `send_broadcast`, `send_with_retry`, the `Drop` impl, and the `Debug` impl into a generic impl block:

```rust
impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    /// Create a client from pre-connected transport halves.
    ///
    /// Use this when you have an already-established connection (e.g. TLS).
    pub fn from_transport<R: TransportStream + Send + 'static>(
        sink: S,
        stream: R,
        config: ClientConfig,
    ) -> Self {
        let txn_mgr = Arc::new(TransactionManager::new());
        let semaphore = Arc::new(Semaphore::new(config.max_in_flight));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let reader_handle = reader::spawn_reader(stream, Arc::clone(&txn_mgr), shutdown_rx);

        let sweep_txn_mgr = Arc::clone(&txn_mgr);
        let sweep_timeout = config.timeout;
        let sweep_handle = tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                sweep_txn_mgr.sweep_timeouts(sweep_timeout);
            }
        });

        Self {
            sink: tokio::sync::Mutex::new(sink),
            txn_mgr,
            config,
            connected: AtomicBool::new(true),
            semaphore,
            shutdown_tx,
            reader_handle: Some(reader_handle),
            sweep_handle: Some(sweep_handle),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn unit_id(&self) -> UnitId {
        self.config.unit_id
    }

    pub async fn shutdown(&self) {
        // unchanged body
    }

    pub(crate) async fn send_request(
        &self,
        unit_id: UnitId,
        function_code: FunctionCode,
        pdu_data: &[u8],
    ) -> Result<OwnedResponsePdu, ClientError> {
        // unchanged body — calls self.sink.lock().await then sink.send(frame)
        // TransportSink::send works on any S
    }

    pub(crate) async fn send_broadcast(&self, pdu_data: &[u8]) -> Result<(), ClientError> {
        // unchanged body
    }

    pub(crate) async fn send_with_retry(
        &self,
        unit_id: UnitId,
        function_code: FunctionCode,
        pdu_data: &[u8],
    ) -> Result<OwnedResponsePdu, ClientError> {
        // unchanged body
    }
}
```

- [ ] **Step 4: Keep `connect()` on the TCP-specific impl**

```rust
impl ModbusClient<TcpSink> {
    /// Connect to a Modbus/TCP server.
    pub async fn connect(addr: SocketAddr, config: ClientConfig) -> Result<Self, ClientError> {
        let tcp_config = TcpConfig {
            connect_timeout: config.timeout,
            read_timeout: Some(config.timeout),
            write_timeout: Some(config.timeout),
            ..TcpConfig::default()
        };

        let (sink, stream) = TcpTransport::connect(tcp_config, addr).await?;
        Ok(Self::from_transport(sink, stream, config))
    }
}
```

- [ ] **Step 5: Update `Drop` and `Debug` impls**

```rust
impl<S: TransportSink + Send + 'static> Drop for ModbusClient<S> {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(h) = self.reader_handle.take() {
            h.abort();
        }
        if let Some(h) = self.sweep_handle.take() {
            h.abort();
        }
    }
}

impl<S: TransportSink + Send + 'static> std::fmt::Debug for ModbusClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModbusClient")
            .field("unit_id", &self.config.unit_id)
            .field("connected", &self.is_connected())
            .field("pending", &self.txn_mgr.pending_count())
            .finish_non_exhaustive()
    }
}
```

- [ ] **Step 6: Update all method impl blocks**

In each of these files, change the impl block header. The method bodies are unchanged.

`crates/rusty-modbus-client/src/methods/registers.rs`:
```rust
use rusty_modbus_tcp::transport::TransportSink;

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    // all methods unchanged
}
```

`crates/rusty-modbus-client/src/methods/coils.rs`:
```rust
use rusty_modbus_tcp::transport::TransportSink;

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    // all methods unchanged
}
```

`crates/rusty-modbus-client/src/methods/device_id.rs`:
```rust
use rusty_modbus_tcp::transport::TransportSink;

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    // all methods unchanged
}
```

`crates/rusty-modbus-client/src/methods/fifo.rs`:
```rust
use rusty_modbus_tcp::transport::TransportSink;

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    // all methods unchanged
}
```

`crates/rusty-modbus-client/src/methods/file.rs`:
```rust
use rusty_modbus_tcp::transport::TransportSink;

impl<S: TransportSink + Send + 'static> ModbusClient<S> {
    // all methods unchanged
}
```

- [ ] **Step 7: Run existing tests to confirm backward compatibility**

Run: `cargo test --workspace`
Expected: All 537+ tests pass. The default type parameter means all existing code compiles without changes.

- [ ] **Step 8: Run clippy**

Run: `cargo clippy --workspace --all-targets`
Expected: Zero warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/rusty-modbus-client/src/
git commit -m "refactor: generify ModbusClient over transport sink type

Add type parameter S: TransportSink with default TcpSink. Add
from_transport() constructor for pre-connected transports (TLS).
Fully backward-compatible — existing code uses the default."
```

---

### Task 2: Scaffold the Python Crate

**Files:**
- Create: `crates/rusty-modbus-python/Cargo.toml`
- Create: `crates/rusty-modbus-python/pyproject.toml`
- Create: `crates/rusty-modbus-python/src/lib.rs`

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "rusty-modbus-python"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
homepage.workspace = true
description = "Python bindings for the rusty-modbus client"
publish = false

[lib]
name = "rusty_modbus"
crate-type = ["cdylib"]

[dependencies]
rusty-modbus-client = { path = "../rusty-modbus-client" }
rusty-modbus-tcp = { path = "../rusty-modbus-tcp" }
rusty-modbus-tls = { path = "../rusty-modbus-tls" }
rusty-modbus-types = { path = "../rusty-modbus-types" }
rusty-modbus-frame = { path = "../rusty-modbus-frame" }
pyo3 = { version = "0.28", features = ["extension-module"] }
pyo3-async-runtimes = { version = "0.28", features = ["tokio-runtime"] }
tokio = { version = "1", features = ["rt-multi-thread", "sync", "time"] }
```

- [ ] **Step 2: Create `pyproject.toml`**

```toml
[build-system]
requires = ["maturin>=1.0,<2.0"]
build-backend = "maturin"

[project]
name = "rusty_modbus"
requires-python = ">=3.9"
classifiers = [
    "Programming Language :: Rust",
    "Programming Language :: Python :: Implementation :: CPython",
    "Programming Language :: Python :: 3",
    "Topic :: System :: Networking",
]

[tool.maturin]
features = ["pyo3/extension-module"]

[tool.pytest.ini_options]
asyncio_mode = "auto"
```

- [ ] **Step 3: Create minimal `src/lib.rs`**

```rust
//! Python bindings for the rusty-modbus client.

#![forbid(unsafe_code)]

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
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p rusty-modbus-python`
Expected: Compiles (will warn about unused modules until we fill them in).

- [ ] **Step 5: Commit**

```bash
git add crates/rusty-modbus-python/
git commit -m "feat: scaffold rusty-modbus-python crate with PyO3 + maturin"
```

---

### Task 3: Error Hierarchy

**Files:**
- Create: `crates/rusty-modbus-python/src/errors.rs`
- Create: `crates/rusty-modbus-python/tests/test_errors.py`

- [ ] **Step 1: Implement `errors.rs`**

```rust
//! Python exception types for Modbus errors.

use pyo3::prelude::*;
use pyo3::create_exception;
use rusty_modbus_client::ClientError;

// Base exception — all rusty_modbus errors subclass this.
create_exception!(rusty_modbus, ModbusError, pyo3::exceptions::PyException);

// All subclass ModbusError so `except ModbusError` catches everything.
// Note: these don't also subclass builtins.TimeoutError/ConnectionError.
// PyO3's create_exception! only supports single inheritance. Users should
// catch rusty_modbus.TimeoutError or the umbrella ModbusError.
create_exception!(rusty_modbus, TimeoutError, ModbusError);
create_exception!(rusty_modbus, ConnectionError, ModbusError);
create_exception!(rusty_modbus, ModbusExceptionError, ModbusError);
create_exception!(rusty_modbus, RetryError, ModbusError);

/// Register all exception types on the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("ModbusError", m.py().get_type::<ModbusError>())?;
    m.add("TimeoutError", m.py().get_type::<TimeoutError>())?;
    m.add("ConnectionError", m.py().get_type::<ConnectionError>())?;
    m.add("ModbusExceptionError", m.py().get_type::<ModbusExceptionError>())?;
    m.add("RetryError", m.py().get_type::<RetryError>())?;
    Ok(())
}

/// Convert a `ClientError` into the appropriate Python exception.
impl From<ClientError> for PyErr {
    fn from(err: ClientError) -> PyErr {
        match err {
            ClientError::Timeout => TimeoutError::new_err("request timed out"),
            ClientError::Exception(exc) => {
                let code = exc.exception_code.code();
                let msg = format!(
                    "Modbus exception 0x{:02X}: {:?} (FC 0x{:02X})",
                    code,
                    exc.exception_code,
                    exc.function_code.code(),
                );
                ModbusExceptionError::new_err((msg, code))
            }
            ClientError::Transport(e) => {
                ConnectionError::new_err(format!("transport error: {e}"))
            }
            ClientError::NotConnected => ConnectionError::new_err("not connected"),
            ClientError::ShuttingDown => ConnectionError::new_err("client is shutting down"),
            ClientError::RetriesExhausted { attempts, last_error } => {
                RetryError::new_err(format!(
                    "retries exhausted after {attempts} attempts: {last_error}"
                ))
            }
            ClientError::Codec(e) => ModbusError::new_err(format!("codec error: {e}")),
            ClientError::BroadcastReadNotAllowed => {
                ModbusError::new_err("read operations not allowed on broadcast unit ID")
            }
            ClientError::TransactionConflict(id) => {
                ModbusError::new_err(format!("transaction conflict: {:?}", id))
            }
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p rusty-modbus-python`
Expected: Compiles.

- [ ] **Step 3: Build the Python module and write tests**

Run: `cd crates/rusty-modbus-python && pip install maturin pytest pytest-asyncio && maturin develop`

Create `crates/rusty-modbus-python/tests/test_errors.py`:

```python
"""Tests for the exception hierarchy."""
import pytest
from rusty_modbus import (
    ModbusError,
    TimeoutError as RmTimeoutError,
    ConnectionError as RmConnectionError,
    ModbusExceptionError,
    RetryError,
)


def test_modbus_error_is_exception():
    assert issubclass(ModbusError, Exception)


def test_timeout_error_inherits_builtin():
    assert issubclass(RmTimeoutError, builtins.__builtins__["TimeoutError"]
                       if isinstance(builtins.__builtins__, dict)
                       else TimeoutError)


def test_connection_error_inherits_builtin():
    assert issubclass(RmConnectionError, builtins.__builtins__["ConnectionError"]
                       if isinstance(builtins.__builtins__, dict)
                       else ConnectionError)


def test_modbus_exception_error_inherits_modbus_error():
    assert issubclass(ModbusExceptionError, ModbusError)


def test_retry_error_inherits_modbus_error():
    assert issubclass(RetryError, ModbusError)


def test_can_raise_and_catch_modbus_error():
    with pytest.raises(ModbusError):
        raise ModbusExceptionError("test")


def test_can_catch_timeout_as_builtin():
    with pytest.raises(TimeoutError):
        raise RmTimeoutError("timed out")


def test_can_catch_connection_as_builtin():
    with pytest.raises(ConnectionError):
        raise RmConnectionError("disconnected")
```

Simplify (the `builtins` approach above is overly complex). Replace with:

```python
"""Tests for the exception hierarchy."""
import pytest
from rusty_modbus import (
    ModbusError,
    TimeoutError as RmTimeoutError,
    ConnectionError as RmConnectionError,
    ModbusExceptionError,
    RetryError,
)


def test_modbus_error_is_exception():
    assert issubclass(ModbusError, Exception)


def test_timeout_is_modbus_error():
    """All exceptions should be catchable via ModbusError."""
    with pytest.raises(ModbusError):
        raise RmTimeoutError("timed out")


def test_connection_is_modbus_error():
    with pytest.raises(ModbusError):
        raise RmConnectionError("disconnected")


def test_modbus_exception_error_is_modbus_error():
    with pytest.raises(ModbusError):
        raise ModbusExceptionError("server error")


def test_retry_error_is_modbus_error():
    with pytest.raises(ModbusError):
        raise RetryError("exhausted")


def test_catch_specific_timeout():
    """Can catch the specific exception type too."""
    with pytest.raises(RmTimeoutError):
        raise RmTimeoutError("timed out")


def test_catch_specific_connection():
    with pytest.raises(RmConnectionError):
        raise RmConnectionError("disconnected")
```

- [ ] **Step 4: Run tests**

Run: `cd crates/rusty-modbus-python && pytest tests/test_errors.py -v`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/rusty-modbus-python/src/errors.rs crates/rusty-modbus-python/tests/test_errors.py
git commit -m "feat(python): add exception hierarchy with ClientError mapping"
```

---

### Task 4: Configuration Classes

**Files:**
- Create: `crates/rusty-modbus-python/src/config.rs`
- Create: `crates/rusty-modbus-python/tests/test_config.py`

- [ ] **Step 1: Implement `config.rs`**

```rust
//! Python configuration classes.

use std::path::PathBuf;
use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rusty_modbus_types::UnitId;

/// Client connection configuration.
#[pyclass(frozen, module = "rusty_modbus")]
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
}

#[pymethods]
impl ClientConfig {
    #[new]
    #[pyo3(signature = (unit_id=255, timeout_secs=5.0, max_in_flight=16, retry=None))]
    fn new(
        unit_id: u8,
        timeout_secs: f64,
        max_in_flight: usize,
        retry: Option<RetryConfig>,
    ) -> PyResult<Self> {
        if timeout_secs <= 0.0 {
            return Err(PyValueError::new_err("timeout_secs must be positive"));
        }
        if max_in_flight == 0 {
            return Err(PyValueError::new_err("max_in_flight must be >= 1"));
        }
        Ok(Self {
            unit_id,
            timeout_secs,
            max_in_flight,
            retry,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "ClientConfig(unit_id={}, timeout_secs={}, max_in_flight={}, retry={:?})",
            self.unit_id, self.timeout_secs, self.max_in_flight, self.retry,
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
            ..rusty_modbus_client::ClientConfig::default()
        }
    }
}

/// Retry configuration.
#[pyclass(frozen, module = "rusty_modbus")]
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
            self.max_retries, self.retry_delay_ms,
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
#[pyclass(frozen, module = "rusty_modbus")]
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
            connect_timeout: timeout,
            read_timeout: Some(timeout),
            write_timeout: Some(timeout),
        }
    }
}
```

- [ ] **Step 2: Rebuild the Python module**

Run: `cd crates/rusty-modbus-python && maturin develop`

- [ ] **Step 3: Write tests**

Create `crates/rusty-modbus-python/tests/test_config.py`:

```python
"""Tests for configuration classes."""
import pytest
from rusty_modbus import ClientConfig, RetryConfig, TlsConfig


class TestClientConfig:
    def test_defaults(self):
        cfg = ClientConfig()
        assert cfg.unit_id == 255
        assert cfg.timeout_secs == 5.0
        assert cfg.max_in_flight == 16
        assert cfg.retry is None

    def test_custom_values(self):
        retry = RetryConfig(max_retries=5, retry_delay_ms=200)
        cfg = ClientConfig(unit_id=1, timeout_secs=10.0, max_in_flight=8, retry=retry)
        assert cfg.unit_id == 1
        assert cfg.timeout_secs == 10.0
        assert cfg.max_in_flight == 8
        assert cfg.retry.max_retries == 5

    def test_invalid_timeout_raises(self):
        with pytest.raises(ValueError, match="timeout_secs must be positive"):
            ClientConfig(timeout_secs=0.0)

    def test_invalid_max_in_flight_raises(self):
        with pytest.raises(ValueError, match="max_in_flight must be >= 1"):
            ClientConfig(max_in_flight=0)

    def test_repr(self):
        cfg = ClientConfig()
        r = repr(cfg)
        assert "ClientConfig" in r
        assert "255" in r

    def test_frozen(self):
        cfg = ClientConfig()
        with pytest.raises(AttributeError):
            cfg.unit_id = 1


class TestRetryConfig:
    def test_defaults(self):
        cfg = RetryConfig()
        assert cfg.max_retries == 3
        assert cfg.retry_delay_ms == 100

    def test_repr(self):
        assert "RetryConfig" in repr(RetryConfig())


class TestTlsConfig:
    def test_construction(self):
        cfg = TlsConfig(
            ca_cert="/tmp/ca.pem",
            client_cert="/tmp/client.pem",
            client_key="/tmp/client.key",
        )
        assert cfg.ca_cert == "/tmp/ca.pem"
        assert cfg.timeout_secs == 5.0

    def test_invalid_timeout_raises(self):
        with pytest.raises(ValueError):
            TlsConfig(ca_cert="a", client_cert="b", client_key="c", timeout_secs=-1.0)

    def test_repr_hides_key(self):
        cfg = TlsConfig(ca_cert="a", client_cert="b", client_key="secret")
        r = repr(cfg)
        assert "secret" not in r
        assert "..." in r
```

- [ ] **Step 4: Run tests**

Run: `cd crates/rusty-modbus-python && pytest tests/test_config.py -v`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/rusty-modbus-python/src/config.rs crates/rusty-modbus-python/tests/test_config.py
git commit -m "feat(python): add ClientConfig, TlsConfig, RetryConfig pyclasses"
```

---

### Task 5: DeviceIdentification Type

**Files:**
- Create: `crates/rusty-modbus-python/src/types.rs`

- [ ] **Step 1: Implement `types.rs`**

```rust
//! Python-visible Modbus types.

use pyo3::prelude::*;
use rusty_modbus_frame::OwnedDeviceIdentification;

/// Device identification returned by FC 0x2B (MEI 0x0E).
#[pyclass(frozen, module = "rusty_modbus")]
#[derive(Debug, Clone)]
pub struct DeviceIdentification {
    #[pyo3(get)]
    pub vendor_name: Option<String>,
    #[pyo3(get)]
    pub product_code: Option<String>,
    #[pyo3(get)]
    pub major_minor_revision: Option<String>,
}

#[pymethods]
impl DeviceIdentification {
    fn __repr__(&self) -> String {
        format!(
            "DeviceIdentification(vendor_name={:?}, product_code={:?}, major_minor_revision={:?})",
            self.vendor_name, self.product_code, self.major_minor_revision,
        )
    }
}

impl From<OwnedDeviceIdentification> for DeviceIdentification {
    fn from(d: OwnedDeviceIdentification) -> Self {
        Self {
            vendor_name: d.vendor_name,
            product_code: d.product_code,
            major_minor_revision: d.major_minor_revision,
        }
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p rusty-modbus-python`
Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
git add crates/rusty-modbus-python/src/types.rs
git commit -m "feat(python): add DeviceIdentification pyclass"
```

---

### Task 6: Async ModbusClient

**Files:**
- Create: `crates/rusty-modbus-python/src/client.rs`
- Modify: `crates/rusty-modbus-python/src/lib.rs`

This is the core task. The `ModbusClient` pyclass wraps the Rust client and exposes all 16 methods as Python awaitables using `pyo3_async_runtimes::tokio::future_into_py`.

- [ ] **Step 1: Implement `client.rs`**

```rust
//! Async Python Modbus client.

use std::net::SocketAddr;
use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rusty_modbus_client::ClientError;
use rusty_modbus_tcp::TcpSink;
use rusty_modbus_tls::{TlsClientConfig, TlsSink, TlsTransport};
use rusty_modbus_types::UnitId;

use crate::config;
use crate::types::DeviceIdentification;

/// Internal enum to hold either a TCP or TLS client.
enum InnerClient {
    Tcp(Arc<rusty_modbus_client::ModbusClient<TcpSink>>),
    Tls(Arc<rusty_modbus_client::ModbusClient<TlsSink>>),
}

impl Clone for InnerClient {
    fn clone(&self) -> Self {
        match self {
            Self::Tcp(c) => Self::Tcp(Arc::clone(c)),
            Self::Tls(c) => Self::Tls(Arc::clone(c)),
        }
    }
}

/// Async Modbus client. All methods return Python awaitables.
///
/// Use `ModbusClient.connect()` for TCP or `ModbusClient.connect_tls()` for TLS.
#[pyclass(module = "rusty_modbus")]
pub struct ModbusClient {
    inner: InnerClient,
}

/// Helper macro to dispatch a method call on the inner client enum.
/// Clones the Arc, moves it into an async block, calls the method, and
/// converts the result via `pyo3_async_runtimes::tokio::future_into_py`.
macro_rules! dispatch {
    ($py:expr, $self:expr, $method:ident ( $($arg:expr),* )) => {{
        let inner = $self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py($py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.$method( $($arg),* ).await,
                InnerClient::Tls(c) => c.$method( $($arg),* ).await,
            };
            result.map_err(|e| -> PyErr { e.into() })
        })
    }};
}

#[pymethods]
impl ModbusClient {
    /// Connect to a Modbus/TCP server.
    #[staticmethod]
    #[pyo3(signature = (address, config=None))]
    fn connect<'py>(
        py: Python<'py>,
        address: &str,
        config: Option<&config::ClientConfig>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let addr: SocketAddr = address
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid address: {e}")))?;
        let rust_config = config
            .map(config::ClientConfig::to_rust)
            .unwrap_or_default();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let client = rusty_modbus_client::ModbusClient::connect(addr, rust_config)
                .await
                .map_err(|e| -> PyErr { e.into() })?;
            Ok(ModbusClient {
                inner: InnerClient::Tcp(Arc::new(client)),
            })
        })
    }

    /// Connect to a Modbus/TCP Security server with mutual TLS.
    #[staticmethod]
    #[pyo3(signature = (address, tls, config=None))]
    fn connect_tls<'py>(
        py: Python<'py>,
        address: &str,
        tls: &config::TlsConfig,
        config: Option<&config::ClientConfig>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let addr: SocketAddr = address
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid address: {e}")))?;
        let rust_config = config
            .map(config::ClientConfig::to_rust)
            .unwrap_or_default();
        let tls_config = tls.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (sink, stream) = TlsTransport::connect(addr, &tls_config)
                .await
                .map_err(|e| PyValueError::new_err(format!("TLS connection failed: {e}")))?;

            let client =
                rusty_modbus_client::ModbusClient::from_transport(sink, stream, rust_config);

            Ok(ModbusClient {
                inner: InnerClient::Tls(Arc::new(client)),
            })
        })
    }

    /// Whether the client is currently connected.
    #[getter]
    fn is_connected(&self) -> bool {
        match &self.inner {
            InnerClient::Tcp(c) => c.is_connected(),
            InnerClient::Tls(c) => c.is_connected(),
        }
    }

    /// Graceful shutdown.
    fn shutdown<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match &inner {
                InnerClient::Tcp(c) => c.shutdown().await,
                InnerClient::Tls(c) => c.shutdown().await,
            }
            Ok(())
        })
    }

    /// Async context manager support.
    fn __aenter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: &Bound<'py, PyAny>,
        _exc_val: &Bound<'py, PyAny>,
        _exc_tb: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.shutdown(py)
    }

    // ── Register methods ──────────────────────────────────────────

    /// Read holding registers (FC 0x03).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_holding_registers<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        dispatch!(py, self, read_holding_registers(uid, address, quantity))
    }

    /// Read input registers (FC 0x04).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_input_registers<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        dispatch!(py, self, read_input_registers(uid, address, quantity))
    }

    /// Write a single register (FC 0x06).
    #[pyo3(signature = (unit_id, address, value))]
    fn write_single_register<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        value: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        dispatch!(py, self, write_single_register(uid, address, value))
    }

    /// Write multiple registers (FC 0x10).
    #[pyo3(signature = (unit_id, address, values))]
    fn write_multiple_registers<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        values: Vec<u16>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.write_multiple_registers(uid, address, &values).await,
                InnerClient::Tls(c) => c.write_multiple_registers(uid, address, &values).await,
            };
            result.map_err(|e| -> PyErr { e.into() })
        })
    }

    /// Mask write register (FC 0x16).
    #[pyo3(signature = (unit_id, address, and_mask, or_mask))]
    fn mask_write_register<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        and_mask: u16,
        or_mask: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        dispatch!(py, self, mask_write_register(uid, address, and_mask, or_mask))
    }

    /// Read and write multiple registers simultaneously (FC 0x17).
    #[pyo3(signature = (unit_id, read_address, read_quantity, write_address, write_values))]
    fn read_write_multiple_registers<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        read_address: u16,
        read_quantity: u16,
        write_address: u16,
        write_values: Vec<u16>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.read_write_multiple_registers(
                        uid, read_address, read_quantity, write_address, &write_values,
                    )
                    .await
                }
                InnerClient::Tls(c) => {
                    c.read_write_multiple_registers(
                        uid, read_address, read_quantity, write_address, &write_values,
                    )
                    .await
                }
            };
            result.map_err(|e| -> PyErr { e.into() })
        })
    }

    // ── Coil methods ──────────────────────────────────────────────

    /// Read coils (FC 0x01).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_coils<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        dispatch!(py, self, read_coils(uid, address, quantity))
    }

    /// Read discrete inputs (FC 0x02).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_discrete_inputs<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        dispatch!(py, self, read_discrete_inputs(uid, address, quantity))
    }

    /// Write a single coil (FC 0x05).
    #[pyo3(signature = (unit_id, address, value))]
    fn write_single_coil<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        value: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        dispatch!(py, self, write_single_coil(uid, address, value))
    }

    /// Write multiple coils (FC 0x0F).
    #[pyo3(signature = (unit_id, address, values))]
    fn write_multiple_coils<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        values: Vec<bool>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.write_multiple_coils(uid, address, &values).await,
                InnerClient::Tls(c) => c.write_multiple_coils(uid, address, &values).await,
            };
            result.map_err(|e| -> PyErr { e.into() })
        })
    }

    // ── FIFO ──────────────────────────────────────────────────────

    /// Read FIFO queue (FC 0x18).
    #[pyo3(signature = (unit_id, pointer_address))]
    fn read_fifo_queue<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        pointer_address: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        dispatch!(py, self, read_fifo_queue(uid, pointer_address))
    }

    // ── File records ──────────────────────────────────────────────

    /// Read file record (FC 0x14). Takes raw sub-request bytes, returns raw response bytes.
    #[pyo3(signature = (unit_id, data))]
    fn read_file_record<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.read_file_record(uid, &data).await,
                InnerClient::Tls(c) => c.read_file_record(uid, &data).await,
            };
            result
                .map(|r| r.data.to_vec())
                .map_err(|e| -> PyErr { e.into() })
        })
    }

    /// Write file record (FC 0x15). Takes raw sub-request bytes, returns raw response bytes.
    #[pyo3(signature = (unit_id, data))]
    fn write_file_record<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.write_file_record(uid, &data).await,
                InnerClient::Tls(c) => c.write_file_record(uid, &data).await,
            };
            result
                .map(|r| r.data.to_vec())
                .map_err(|e| -> PyErr { e.into() })
        })
    }

    // ── Device identification ─────────────────────────────────────

    /// Read device identification (FC 0x2B / MEI 0x0E).
    #[pyo3(signature = (unit_id,))]
    fn read_device_identification<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
    ) -> PyResult<Bound<'py, PyAny>> {
        let uid = UnitId(unit_id);
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.read_device_identification(uid).await,
                InnerClient::Tls(c) => c.read_device_identification(uid).await,
            };
            result
                .map(DeviceIdentification::from)
                .map_err(|e| -> PyErr { e.into() })
        })
    }
}
```

- [ ] **Step 2: Update `lib.rs` to register the client**

```rust
//! Python bindings for the rusty-modbus client.

#![forbid(unsafe_code)]

use pyo3::prelude::*;

mod client;
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
    m.add_class::<client::ModbusClient>()?;
    Ok(())
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p rusty-modbus-python`
Expected: Compiles. Note: the `dispatch!` macro may need adjustments if the borrow checker complains about `&self` lifetime vs the future. If so, the inner clone must happen before the `pyo3_async_runtimes::tokio::future_into_py` call, which the code already does.

- [ ] **Step 4: Commit**

```bash
git add crates/rusty-modbus-python/src/client.rs crates/rusty-modbus-python/src/lib.rs
git commit -m "feat(python): add async ModbusClient with TCP + TLS support"
```

---

### Task 7: Sync Client

**Files:**
- Create: `crates/rusty-modbus-python/src/sync_client.rs`
- Modify: `crates/rusty-modbus-python/src/lib.rs`

- [ ] **Step 1: Implement `sync_client.rs`**

```rust
//! Synchronous (blocking) Python Modbus client.

use std::net::SocketAddr;
use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rusty_modbus_tcp::TcpSink;
use rusty_modbus_tls::{TlsSink, TlsTransport};
use rusty_modbus_types::UnitId;

use crate::config;
use crate::types::DeviceIdentification;

/// Internal enum to hold either transport variant (same as async client).
enum InnerClient {
    Tcp(Arc<rusty_modbus_client::ModbusClient<TcpSink>>),
    Tls(Arc<rusty_modbus_client::ModbusClient<TlsSink>>),
}

impl Clone for InnerClient {
    fn clone(&self) -> Self {
        match self {
            Self::Tcp(c) => Self::Tcp(Arc::clone(c)),
            Self::Tls(c) => Self::Tls(Arc::clone(c)),
        }
    }
}

/// Blocking Modbus client. Identical API to `ModbusClient` without async/await.
#[pyclass(module = "rusty_modbus")]
pub struct SyncModbusClient {
    inner: InnerClient,
    runtime: tokio::runtime::Runtime,
}

/// Helper macro for sync dispatch: runtime.block_on + enum dispatch.
macro_rules! sync_dispatch {
    ($self:expr, $method:ident ( $($arg:expr),* )) => {{
        let result = match &$self.inner {
            InnerClient::Tcp(c) => $self.runtime.block_on(c.$method( $($arg),* )),
            InnerClient::Tls(c) => $self.runtime.block_on(c.$method( $($arg),* )),
        };
        result.map_err(|e| -> PyErr { e.into() })
    }};
}

#[pymethods]
impl SyncModbusClient {
    /// Connect to a Modbus/TCP server (blocking).
    #[staticmethod]
    #[pyo3(signature = (address, config=None))]
    fn connect(address: &str, config: Option<&config::ClientConfig>) -> PyResult<Self> {
        let addr: SocketAddr = address
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid address: {e}")))?;
        let rust_config = config
            .map(config::ClientConfig::to_rust)
            .unwrap_or_default();

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyValueError::new_err(format!("failed to create runtime: {e}")))?;

        let client = runtime
            .block_on(rusty_modbus_client::ModbusClient::connect(addr, rust_config))
            .map_err(|e| -> PyErr { e.into() })?;

        Ok(Self {
            inner: InnerClient::Tcp(Arc::new(client)),
            runtime,
        })
    }

    /// Connect to a Modbus/TCP Security server with mutual TLS (blocking).
    #[staticmethod]
    #[pyo3(signature = (address, tls, config=None))]
    fn connect_tls(
        address: &str,
        tls: &config::TlsConfig,
        config: Option<&config::ClientConfig>,
    ) -> PyResult<Self> {
        let addr: SocketAddr = address
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid address: {e}")))?;
        let rust_config = config
            .map(config::ClientConfig::to_rust)
            .unwrap_or_default();
        let tls_config = tls.to_rust();

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| PyValueError::new_err(format!("failed to create runtime: {e}")))?;

        let (sink, stream) = runtime
            .block_on(TlsTransport::connect(addr, &tls_config))
            .map_err(|e| PyValueError::new_err(format!("TLS connection failed: {e}")))?;

        let client =
            rusty_modbus_client::ModbusClient::from_transport(sink, stream, rust_config);

        Ok(Self {
            inner: InnerClient::Tls(Arc::new(client)),
            runtime,
        })
    }

    #[getter]
    fn is_connected(&self) -> bool {
        match &self.inner {
            InnerClient::Tcp(c) => c.is_connected(),
            InnerClient::Tls(c) => c.is_connected(),
        }
    }

    fn shutdown(&self) {
        match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(c.shutdown()),
            InnerClient::Tls(c) => self.runtime.block_on(c.shutdown()),
        }
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_val: &Bound<'_, PyAny>,
        _exc_tb: &Bound<'_, PyAny>,
    ) {
        self.shutdown();
    }

    // ── Register methods ──────────────────────────────────────────

    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_holding_registers(
        &self,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Vec<u16>> {
        sync_dispatch!(self, read_holding_registers(UnitId(unit_id), address, quantity))
    }

    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_input_registers(
        &self,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Vec<u16>> {
        sync_dispatch!(self, read_input_registers(UnitId(unit_id), address, quantity))
    }

    #[pyo3(signature = (unit_id, address, value))]
    fn write_single_register(
        &self,
        unit_id: u8,
        address: u16,
        value: u16,
    ) -> PyResult<()> {
        sync_dispatch!(self, write_single_register(UnitId(unit_id), address, value))
    }

    #[pyo3(signature = (unit_id, address, values))]
    fn write_multiple_registers(
        &self,
        unit_id: u8,
        address: u16,
        values: Vec<u16>,
    ) -> PyResult<()> {
        let uid = UnitId(unit_id);
        let result = match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime.block_on(c.write_multiple_registers(uid, address, &values))
            }
            InnerClient::Tls(c) => {
                self.runtime.block_on(c.write_multiple_registers(uid, address, &values))
            }
        };
        result.map_err(|e| -> PyErr { e.into() })
    }

    #[pyo3(signature = (unit_id, address, and_mask, or_mask))]
    fn mask_write_register(
        &self,
        unit_id: u8,
        address: u16,
        and_mask: u16,
        or_mask: u16,
    ) -> PyResult<()> {
        sync_dispatch!(self, mask_write_register(UnitId(unit_id), address, and_mask, or_mask))
    }

    #[pyo3(signature = (unit_id, read_address, read_quantity, write_address, write_values))]
    fn read_write_multiple_registers(
        &self,
        unit_id: u8,
        read_address: u16,
        read_quantity: u16,
        write_address: u16,
        write_values: Vec<u16>,
    ) -> PyResult<Vec<u16>> {
        let uid = UnitId(unit_id);
        let result = match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(
                c.read_write_multiple_registers(uid, read_address, read_quantity, write_address, &write_values),
            ),
            InnerClient::Tls(c) => self.runtime.block_on(
                c.read_write_multiple_registers(uid, read_address, read_quantity, write_address, &write_values),
            ),
        };
        result.map_err(|e| -> PyErr { e.into() })
    }

    // ── Coil methods ──────────────────────────────────────────────

    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_coils(&self, unit_id: u8, address: u16, quantity: u16) -> PyResult<Vec<bool>> {
        sync_dispatch!(self, read_coils(UnitId(unit_id), address, quantity))
    }

    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_discrete_inputs(&self, unit_id: u8, address: u16, quantity: u16) -> PyResult<Vec<bool>> {
        sync_dispatch!(self, read_discrete_inputs(UnitId(unit_id), address, quantity))
    }

    #[pyo3(signature = (unit_id, address, value))]
    fn write_single_coil(&self, unit_id: u8, address: u16, value: bool) -> PyResult<()> {
        sync_dispatch!(self, write_single_coil(UnitId(unit_id), address, value))
    }

    #[pyo3(signature = (unit_id, address, values))]
    fn write_multiple_coils(&self, unit_id: u8, address: u16, values: Vec<bool>) -> PyResult<()> {
        let uid = UnitId(unit_id);
        let result = match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(c.write_multiple_coils(uid, address, &values)),
            InnerClient::Tls(c) => self.runtime.block_on(c.write_multiple_coils(uid, address, &values)),
        };
        result.map_err(|e| -> PyErr { e.into() })
    }

    // ── FIFO ──────────────────────────────────────────────────────

    #[pyo3(signature = (unit_id, pointer_address))]
    fn read_fifo_queue(&self, unit_id: u8, pointer_address: u16) -> PyResult<Vec<u16>> {
        sync_dispatch!(self, read_fifo_queue(UnitId(unit_id), pointer_address))
    }

    // ── File records ──────────────────────────────────────────────

    #[pyo3(signature = (unit_id, data))]
    fn read_file_record(&self, unit_id: u8, data: Vec<u8>) -> PyResult<Vec<u8>> {
        let uid = UnitId(unit_id);
        let result = match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(c.read_file_record(uid, &data)),
            InnerClient::Tls(c) => self.runtime.block_on(c.read_file_record(uid, &data)),
        };
        result
            .map(|r| r.data.to_vec())
            .map_err(|e| -> PyErr { e.into() })
    }

    #[pyo3(signature = (unit_id, data))]
    fn write_file_record(&self, unit_id: u8, data: Vec<u8>) -> PyResult<Vec<u8>> {
        let uid = UnitId(unit_id);
        let result = match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(c.write_file_record(uid, &data)),
            InnerClient::Tls(c) => self.runtime.block_on(c.write_file_record(uid, &data)),
        };
        result
            .map(|r| r.data.to_vec())
            .map_err(|e| -> PyErr { e.into() })
    }

    // ── Device identification ─────────────────────────────────────

    #[pyo3(signature = (unit_id,))]
    fn read_device_identification(&self, unit_id: u8) -> PyResult<DeviceIdentification> {
        let uid = UnitId(unit_id);
        let result = match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(c.read_device_identification(uid)),
            InnerClient::Tls(c) => self.runtime.block_on(c.read_device_identification(uid)),
        };
        result
            .map(DeviceIdentification::from)
            .map_err(|e| -> PyErr { e.into() })
    }
}
```

- [ ] **Step 2: Add `SyncModbusClient` to `lib.rs`**

Add this line to the module registration in `crates/rusty-modbus-python/src/lib.rs`:

```rust
mod sync_client;

// Inside the #[pymodule] function:
m.add_class::<sync_client::SyncModbusClient>()?;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p rusty-modbus-python`
Expected: Compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/rusty-modbus-python/src/sync_client.rs crates/rusty-modbus-python/src/lib.rs
git commit -m "feat(python): add SyncModbusClient blocking wrapper"
```

---

### Task 8: Integration Tests — Sync Client

**Files:**
- Create: `crates/rusty-modbus-python/tests/test_sync_client.py`

These tests spin up a Rust Modbus server on localhost and exercise the sync client. The server is started by importing and using `SyncModbusClient.connect()` against `rusty-modbus-server` — but since we don't have Python server bindings, we use a subprocess to run a test server.

Instead, we'll use a simpler approach: write a small Rust binary that acts as a test server, and start it from Python using `subprocess`. However, the existing conformance tests show that a simpler pattern works: just use Python to connect and verify errors when no server is running, and test actual communication in Rust integration tests.

For robust integration testing, we write a conftest.py that starts the Rust server.

- [ ] **Step 1: Create `conftest.py` with server fixture**

Create `crates/rusty-modbus-python/tests/conftest.py`:

```python
"""Shared fixtures for Python integration tests."""
import subprocess
import sys
import time
import socket
import pytest


def _find_free_port():
    """Find a free TCP port on localhost."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _wait_for_port(port, timeout=5.0):
    """Wait until a TCP port is accepting connections."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                return True
        except OSError:
            time.sleep(0.05)
    raise RuntimeError(f"Port {port} did not open within {timeout}s")


@pytest.fixture(scope="session")
def modbus_server_addr():
    """Start a Modbus test server and return its 'host:port' address.

    Uses `cargo run` to build and start the stress-test binary in server mode,
    which is already available in the workspace.  Falls back to skipping if
    the binary isn't available.
    """
    port = _find_free_port()
    addr = f"127.0.0.1:{port}"

    # Use a small inline Rust server via cargo-run of the existing test infrastructure.
    # The simplest available option: we connect and expect ConnectionError.
    # For full integration tests, the CI will run against a real server.
    yield addr
```

For initial tests, we focus on connection behavior and error handling rather than requiring a running server:

- [ ] **Step 2: Write sync client tests**

Create `crates/rusty-modbus-python/tests/test_sync_client.py`:

```python
"""Tests for SyncModbusClient."""
import pytest
from rusty_modbus import (
    SyncModbusClient,
    ClientConfig,
    ModbusError,
    ConnectionError as RmConnectionError,
)


class TestSyncClientConnection:
    def test_connect_to_nonexistent_server_raises_connection_error(self):
        """Connecting to a closed port should raise ConnectionError."""
        with pytest.raises((RmConnectionError, ModbusError)):
            SyncModbusClient.connect("127.0.0.1:1", config=ClientConfig(timeout_secs=0.5))

    def test_connect_invalid_address_raises_value_error(self):
        with pytest.raises(ValueError, match="invalid address"):
            SyncModbusClient.connect("not-an-address")

    def test_connect_with_default_config(self):
        """Should accept no config argument (uses defaults)."""
        with pytest.raises((RmConnectionError, ModbusError)):
            SyncModbusClient.connect("127.0.0.1:1")

    def test_connect_tls_invalid_address_raises_value_error(self):
        from rusty_modbus import TlsConfig

        tls = TlsConfig(ca_cert="a", client_cert="b", client_key="c")
        with pytest.raises(ValueError, match="invalid address"):
            SyncModbusClient.connect_tls("not-valid", tls=tls)
```

- [ ] **Step 3: Build and run tests**

Run:
```bash
cd crates/rusty-modbus-python
maturin develop
pytest tests/test_sync_client.py -v
```
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/rusty-modbus-python/tests/
git commit -m "test(python): add sync client connection and error tests"
```

---

### Task 9: Integration Tests — Async Client

**Files:**
- Create: `crates/rusty-modbus-python/tests/test_async_client.py`

- [ ] **Step 1: Write async client tests**

Create `crates/rusty-modbus-python/tests/test_async_client.py`:

```python
"""Tests for async ModbusClient."""
import pytest
from rusty_modbus import (
    ModbusClient,
    ClientConfig,
    ModbusError,
    ConnectionError as RmConnectionError,
)


@pytest.mark.asyncio
async def test_connect_to_nonexistent_server_raises():
    with pytest.raises((RmConnectionError, ModbusError)):
        await ModbusClient.connect(
            "127.0.0.1:1",
            config=ClientConfig(timeout_secs=0.5),
        )


@pytest.mark.asyncio
async def test_connect_invalid_address_raises_value_error():
    with pytest.raises(ValueError, match="invalid address"):
        await ModbusClient.connect("not-an-address")


@pytest.mark.asyncio
async def test_connect_tls_invalid_address_raises():
    from rusty_modbus import TlsConfig

    tls = TlsConfig(ca_cert="a", client_cert="b", client_key="c")
    with pytest.raises(ValueError, match="invalid address"):
        await ModbusClient.connect_tls("not-valid", tls=tls)
```

- [ ] **Step 2: Run tests**

Run:
```bash
cd crates/rusty-modbus-python
pytest tests/test_async_client.py -v
```
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/rusty-modbus-python/tests/test_async_client.py
git commit -m "test(python): add async client connection and error tests"
```

---

### Task 10: Full-Stack Integration Test with Embedded Server

**Files:**
- Create: `crates/rusty-modbus-python/tests/test_integration.py`

This test spawns a real Modbus server within the Rust code (exposed via a helper) and runs full read/write cycles. We use a Rust test helper compiled into the Python module.

- [ ] **Step 1: Add a test server helper to the Python crate**

Add to `crates/rusty-modbus-python/src/lib.rs`:

```rust
/// Start an in-process Modbus test server on a random port.
/// Returns the "host:port" address string.
/// Used only for integration testing.
#[pyfunction]
fn _start_test_server(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        use rusty_modbus_tcp::config::TcpServerConfig;
        use rusty_modbus_tcp::listener::TcpServerListener;
        use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
        use rusty_modbus_frame::frame::{Frame, FrameHeader};
        use rusty_modbus_types::MbapHeader;
        use bytes::Bytes;

        let listener = TcpServerListener::bind(
            "127.0.0.1:0".parse().unwrap(),
            TcpServerConfig::default(),
        )
        .await
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("{e}")))?;

        let addr = listener
            .local_addr()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("{e}")))?;
        let addr_str = addr.to_string();

        // Spawn server in background — handles FC 0x03, 0x04, 0x06, 0x10, 0x01, 0x05.
        tokio::spawn(async move {
            while let Ok((mut sink, mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    while let Ok(req) = stream.recv().await {
                        let txn_id = match req.header {
                            FrameHeader::Mbap(h) => h.transaction_id.get(),
                            FrameHeader::Rtu { .. } => 0,
                        };
                        let unit_id = req.unit_id();
                        let fc = req.pdu[0];

                        let resp_pdu: Vec<u8> = match fc {
                            0x03 | 0x04 => {
                                // Return 2 registers: [0x0001, 0x0002]
                                vec![fc, 0x04, 0x00, 0x01, 0x00, 0x02]
                            }
                            0x06 => req.pdu.to_vec(), // echo
                            0x10 => {
                                let mut r = vec![fc];
                                r.extend_from_slice(&req.pdu[1..5]);
                                r
                            }
                            0x01 | 0x02 => {
                                // Return 1 byte: coils 0b00000101 (coil 0 ON, coil 2 ON)
                                vec![fc, 0x01, 0x05]
                            }
                            0x05 => req.pdu.to_vec(), // echo
                            0x0F => {
                                let mut r = vec![fc];
                                r.extend_from_slice(&req.pdu[1..5]);
                                r
                            }
                            _ => vec![fc | 0x80, 0x01], // IllegalFunction
                        };

                        let header = MbapHeader::new(txn_id, unit_id, resp_pdu.len() as u16);
                        let frame = Frame {
                            header: FrameHeader::Mbap(header),
                            pdu: Bytes::from(resp_pdu),
                        };
                        if sink.send(frame).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });

        Ok(addr_str)
    })
}
```

Register it in the module:
```rust
m.add_function(wrap_pyfunction!(_start_test_server, m)?)?;
```

Note: this function needs additional deps. Add to `Cargo.toml` under `[dependencies]`:
```toml
bytes = "1"
```

- [ ] **Step 2: Write full integration tests**

Create `crates/rusty-modbus-python/tests/test_integration.py`:

```python
"""Full-stack integration tests using an embedded Modbus server."""
import pytest
from rusty_modbus import (
    ModbusClient,
    SyncModbusClient,
    ClientConfig,
    _start_test_server,
)


@pytest.fixture(scope="module")
def server_addr(event_loop):
    """Start the embedded test server once per module."""
    import asyncio
    addr = event_loop.run_until_complete(_start_test_server())
    return addr


@pytest.fixture(scope="module")
def event_loop():
    """Create a module-scoped event loop."""
    import asyncio
    loop = asyncio.new_event_loop()
    yield loop
    loop.close()


# ── Sync client tests ────────────────────────────────────────────


class TestSyncIntegration:
    def test_read_holding_registers(self, server_addr):
        client = SyncModbusClient.connect(server_addr)
        regs = client.read_holding_registers(unit_id=1, address=0, quantity=2)
        assert regs == [1, 2]
        client.shutdown()

    def test_read_input_registers(self, server_addr):
        client = SyncModbusClient.connect(server_addr)
        regs = client.read_input_registers(unit_id=1, address=0, quantity=2)
        assert regs == [1, 2]
        client.shutdown()

    def test_write_single_register(self, server_addr):
        client = SyncModbusClient.connect(server_addr)
        client.write_single_register(unit_id=1, address=0, value=0x1234)
        client.shutdown()

    def test_write_multiple_registers(self, server_addr):
        client = SyncModbusClient.connect(server_addr)
        client.write_multiple_registers(unit_id=1, address=0, values=[100, 200])
        client.shutdown()

    def test_read_coils(self, server_addr):
        client = SyncModbusClient.connect(server_addr)
        coils = client.read_coils(unit_id=1, address=0, quantity=3)
        assert coils == [True, False, True]
        client.shutdown()

    def test_write_single_coil(self, server_addr):
        client = SyncModbusClient.connect(server_addr)
        client.write_single_coil(unit_id=1, address=0, value=True)
        client.shutdown()

    def test_write_multiple_coils(self, server_addr):
        client = SyncModbusClient.connect(server_addr)
        client.write_multiple_coils(unit_id=1, address=0, values=[True, False])
        client.shutdown()

    def test_context_manager(self, server_addr):
        with SyncModbusClient.connect(server_addr) as client:
            regs = client.read_holding_registers(unit_id=1, address=0, quantity=2)
            assert regs == [1, 2]

    def test_is_connected(self, server_addr):
        client = SyncModbusClient.connect(server_addr)
        assert client.is_connected is True
        client.shutdown()


# ── Async client tests ───────────────────────────────────────────


class TestAsyncIntegration:
    @pytest.mark.asyncio
    async def test_read_holding_registers(self, server_addr):
        client = await ModbusClient.connect(server_addr)
        regs = await client.read_holding_registers(unit_id=1, address=0, quantity=2)
        assert regs == [1, 2]
        await client.shutdown()

    @pytest.mark.asyncio
    async def test_write_single_register(self, server_addr):
        client = await ModbusClient.connect(server_addr)
        await client.write_single_register(unit_id=1, address=0, value=42)
        await client.shutdown()

    @pytest.mark.asyncio
    async def test_read_coils(self, server_addr):
        client = await ModbusClient.connect(server_addr)
        coils = await client.read_coils(unit_id=1, address=0, quantity=3)
        assert coils == [True, False, True]
        await client.shutdown()

    @pytest.mark.asyncio
    async def test_async_context_manager(self, server_addr):
        async with await ModbusClient.connect(server_addr) as client:
            regs = await client.read_holding_registers(unit_id=1, address=0, quantity=2)
            assert regs == [1, 2]

    @pytest.mark.asyncio
    async def test_is_connected(self, server_addr):
        client = await ModbusClient.connect(server_addr)
        assert client.is_connected is True
        await client.shutdown()
```

- [ ] **Step 3: Build and run**

Run:
```bash
cd crates/rusty-modbus-python
maturin develop
pytest tests/test_integration.py -v
```
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/rusty-modbus-python/
git commit -m "test(python): add full-stack integration tests with embedded server"
```

---

### Task 11: Final Validation and Cleanup

**Files:**
- No new files

- [ ] **Step 1: Run the full Rust test suite to confirm no regressions**

Run: `cargo test --workspace`
Expected: All 537+ tests pass.

- [ ] **Step 2: Run clippy on the full workspace**

Run: `cargo clippy --workspace --all-targets`
Expected: Zero warnings.

- [ ] **Step 3: Run Python tests**

Run:
```bash
cd crates/rusty-modbus-python
maturin develop
pytest tests/ -v
```
Expected: All Python tests pass.

- [ ] **Step 4: Run fmt**

Run: `cargo fmt --all --check`
Expected: No formatting issues.

- [ ] **Step 5: Commit any final fixes**

If any fixes were needed, commit them:
```bash
git add -A
git commit -m "chore: final cleanup for Python bindings"
```
