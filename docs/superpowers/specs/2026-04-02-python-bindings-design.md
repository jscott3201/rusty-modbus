# Python Bindings Design Spec

## Summary

Add a PyO3-based Python client library (`rusty_modbus`) that wraps the existing Rust Modbus client. Supports TCP and TLS transports, async-first with a sync convenience wrapper. New workspace crate: `crates/rusty-modbus-python/`.

## Decisions

- **Transports**: TCP + TLS. RTU-over-TCP deferred.
- **Async model**: asyncio-native async client + sync blocking wrapper.
- **Package name**: Python package `rusty_modbus`, crate `rusty-modbus-python`.
- **Build tool**: maturin (standard for PyO3 projects).
- **Scope**: Client-only. No server, gateway, pool, or simulator bindings.

## Dependencies

- `pyo3 = "0.28"` with `extension-module` feature
- `pyo3-async-runtimes = "0.28"` with `tokio-runtime` feature
- `tokio = "1"` with `rt-multi-thread`, `sync`, `time` features
- Workspace crates: `rusty-modbus-client`, `rusty-modbus-tcp`, `rusty-modbus-tls`, `rusty-modbus-types`, `rusty-modbus-frame`

## Python API

### Module Exports

```python
from rusty_modbus import (
    ModbusClient,          # async client
    SyncModbusClient,      # blocking wrapper
    ClientConfig,
    TlsConfig,
    RetryConfig,
    DeviceIdentification,
    ModbusError,
    TimeoutError,
    ModbusExceptionError,
    ConnectionError,
    RetryError,
)
```

### ModbusClient (async)

```python
# TCP
client = await ModbusClient.connect("192.168.1.100:502")
client = await ModbusClient.connect("192.168.1.100:502", config=ClientConfig(...))

# TLS
client = await ModbusClient.connect_tls("192.168.1.100:802", tls=TlsConfig(...))
client = await ModbusClient.connect_tls("192.168.1.100:802", tls=TlsConfig(...), config=ClientConfig(...))

# Context manager
async with await ModbusClient.connect("192.168.1.100:502") as client:
    ...
```

#### Register Methods

| Method | Parameters | Returns |
|--------|-----------|---------|
| `read_holding_registers` | `unit_id: int, address: int, quantity: int` | `list[int]` |
| `read_input_registers` | `unit_id: int, address: int, quantity: int` | `list[int]` |
| `write_single_register` | `unit_id: int, address: int, value: int` | `None` |
| `write_multiple_registers` | `unit_id: int, address: int, values: list[int]` | `None` |
| `mask_write_register` | `unit_id: int, address: int, and_mask: int, or_mask: int` | `None` |
| `read_write_multiple_registers` | `unit_id: int, read_address: int, read_quantity: int, write_address: int, write_values: list[int]` | `list[int]` |

#### Coil Methods

| Method | Parameters | Returns |
|--------|-----------|---------|
| `read_coils` | `unit_id: int, address: int, quantity: int` | `list[bool]` |
| `read_discrete_inputs` | `unit_id: int, address: int, quantity: int` | `list[bool]` |
| `write_single_coil` | `unit_id: int, address: int, value: bool` | `None` |
| `write_multiple_coils` | `unit_id: int, address: int, values: list[bool]` | `None` |

#### Other Methods

| Method | Parameters | Returns |
|--------|-----------|---------|
| `read_fifo_queue` | `unit_id: int, pointer_address: int` | `list[int]` |
| `read_file_record` | `unit_id: int, data: bytes` | `bytes` |
| `write_file_record` | `unit_id: int, data: bytes` | `bytes` |
| `read_device_identification` | `unit_id: int` | `DeviceIdentification` |

#### Lifecycle

| Member | Type | Description |
|--------|------|-------------|
| `is_connected` | property `bool` | Connection state |
| `shutdown()` | async method | Graceful shutdown |
| `__aenter__` / `__aexit__` | async context manager | Auto-shutdown on exit |

### SyncModbusClient

Identical method signatures without `async`/`await`. Uses `with` instead of `async with`.

```python
client = SyncModbusClient.connect("192.168.1.100:502")
client = SyncModbusClient.connect_tls("192.168.1.100:802", tls=TlsConfig(...))

with SyncModbusClient.connect("192.168.1.100:502") as client:
    regs = client.read_holding_registers(unit_id=1, address=0, quantity=10)
```

Implementation: owns a `tokio::runtime::Runtime`, calls `runtime.block_on()` for each operation (blocks the calling thread while the tokio thread pool executes the future).

### Configuration

```python
class ClientConfig:
    unit_id: int = 255         # 0xFF = TCP direct device
    timeout_secs: float = 5.0
    max_in_flight: int = 16
    retry: RetryConfig | None = None

class RetryConfig:
    max_retries: int = 3
    retry_delay_ms: int = 100

class TlsConfig:
    ca_cert: str               # path to CA certificate
    client_cert: str           # path to client certificate
    client_key: str            # path to client private key
    timeout_secs: float = 5.0
```

All config classes support `__repr__` for debugging.

### DeviceIdentification

```python
class DeviceIdentification:
    vendor_name: str | None
    product_code: str | None
    major_minor_revision: str | None
```

Read-only properties. Supports `__repr__`.

### Error Hierarchy

```
ModbusError (base exception for all rusty_modbus errors)
├── TimeoutError          (also subclasses builtins.TimeoutError)
├── ModbusExceptionError  (server returned an exception PDU)
│   ├── exception_code: int    (raw code: 0x01-0x0B)
│   └── message: str
├── ConnectionError       (also subclasses builtins.ConnectionError)
└── RetryError
    ├── attempts: int
    └── last_error: ModbusError
```

Mapping from `ClientError` variants:

| ClientError variant | Python exception |
|-------------------|-----------------|
| `Timeout` | `TimeoutError` |
| `Exception(resp)` | `ModbusExceptionError` |
| `Transport(_)` | `ConnectionError` |
| `Codec(_)` | `ModbusError` |
| `NotConnected` | `ConnectionError` |
| `RetriesExhausted{..}` | `RetryError` |
| `BroadcastReadNotAllowed` | `ModbusError` |
| `ShuttingDown` | `ConnectionError` |
| `TransactionConflict(_)` | `ModbusError` |

## Rust Architecture

### File Structure

```
crates/rusty-modbus-python/
├── Cargo.toml
├── pyproject.toml
├── src/
│   ├── lib.rs           # #[pymodule] definition
│   ├── client.rs        # ModbusClient pyclass
│   ├── sync_client.rs   # SyncModbusClient pyclass
│   ├── config.rs        # ClientConfig, TlsConfig, RetryConfig pyclasses
│   ├── types.rs         # DeviceIdentification pyclass
│   └── errors.rs        # Exception hierarchy via create_exception!
```

### Key Implementation Details

**ModbusClient pyclass:**
- Holds `Arc<RustModbusClient>` (the Rust client is `Clone` via inner `Arc`)
- Each async method clones the `Arc`, moves it into a future, calls `pyo3_async_runtimes::tokio::future_into_py()` to return a Python awaitable
- GIL is released during the Rust future execution (I/O happens off the GIL)
- `connect()` and `connect_tls()` are `#[staticmethod]` async methods

**SyncModbusClient pyclass:**
- Owns `tokio::runtime::Runtime` (created once at connect time)
- Owns `Arc<RustModbusClient>` (same inner client)
- Each method calls `self.runtime.block_on(...)` 
- `connect()` and `connect_tls()` are `#[staticmethod]` (synchronous, runtime created inline)

**Config pyclasses:**
- `#[pyclass(frozen)]` — immutable after construction, no borrow-checking overhead
- Constructor validates values (unit_id 0-255, timeout > 0, etc.)
- Internal conversion methods `to_rust()` produce the corresponding Rust config structs

**Error conversion:**
- Single `impl From<ClientError> for PyErr` that pattern-matches and constructs the appropriate Python exception
- `create_exception!` macro for each exception type
- `ModbusExceptionError` stores the raw exception code as an attribute

### Workspace Integration

- Added to workspace `Cargo.toml` members (already `crates/*`)
- Not part of the facade crate feature flags (standalone distribution)
- Not published to crates.io (distributed as a Python wheel via PyPI)
- CI: add a maturin build + pytest job

### Build

```bash
# Development
cd crates/rusty-modbus-python
pip install maturin
maturin develop

# Release wheels
maturin build --release
```

## Out of Scope

- RTU/serial transport
- Server, gateway, pool, simulator bindings
- Connection pooling at the Python level
- Type stubs (.pyi files) — can be added later
- PyPI publishing pipeline — can be added later

## Testing Strategy

- Unit tests in Rust (`#[cfg(test)]` in each module) for config conversion, error mapping
- Python integration tests using pytest + a Rust test server spawned in-process
- Test both async (pytest-asyncio) and sync clients
- Tests live in `crates/rusty-modbus-python/tests/` (Python) alongside Rust tests
