//! Synchronous `SyncModbusClient` — blocking wrapper for use without ``asyncio``.

use std::net::SocketAddr;
use std::sync::Arc;

use pyo3::prelude::*;
use rusty_modbus_client::ModbusClient as RustClient;
use rusty_modbus_tcp::TcpSink;
use rusty_modbus_tls::{TlsRecvStream, TlsSink, TlsTransport};
use rusty_modbus_types::UnitId;
use tokio::runtime::Runtime;

use crate::config::{ClientConfig, TlsConfig};
use crate::errors;
use crate::types::DeviceIdentification;

/// Transport-agnostic handle: either a TCP or TLS inner client.
#[derive(Clone)]
enum InnerClient {
    Tcp(Arc<RustClient<TcpSink>>),
    Tls(Arc<RustClient<TlsSink>>),
}

/// Synchronous (blocking) Modbus client.
///
/// Provides the same methods as [`ModbusClient`](crate::client::ModbusClient)
/// but blocks the calling thread instead of returning awaitables. Suitable for
/// scripts, REPL sessions, and non-async applications.
#[pyclass(module = "rusty_modbus")]
pub struct SyncModbusClient {
    inner: InnerClient,
    runtime: Runtime,
}

#[pymethods]
impl SyncModbusClient {
    // ── construction ────────────────────────────────────────────────

    /// Connect to a Modbus/TCP server (blocking).
    #[staticmethod]
    #[pyo3(signature = (address, config=None))]
    fn connect(py: Python<'_>, address: &str, config: Option<ClientConfig>) -> PyResult<Self> {
        let addr: SocketAddr = address
            .parse()
            .map_err(|e| errors::ConnectionError::new_err(format!("invalid address: {e}")))?;
        let cfg = config.map_or_else(rusty_modbus_client::ClientConfig::default, |c| c.to_rust());

        let runtime = Runtime::new()
            .map_err(|e| errors::ConnectionError::new_err(format!("runtime error: {e}")))?;

        let client = py
            .detach(|| runtime.block_on(RustClient::connect(addr, cfg)))
            .map_err(errors::client_error_to_pyerr)?;

        Ok(Self {
            inner: InnerClient::Tcp(Arc::new(client)),
            runtime,
        })
    }

    /// Connect to a Modbus/TCP Security (TLS) server (blocking).
    #[staticmethod]
    #[pyo3(signature = (address, tls, config=None))]
    fn connect_tls(
        py: Python<'_>,
        address: &str,
        tls: TlsConfig,
        config: Option<ClientConfig>,
    ) -> PyResult<Self> {
        let addr: SocketAddr = address
            .parse()
            .map_err(|e| errors::ConnectionError::new_err(format!("invalid address: {e}")))?;
        let cfg = config.map_or_else(rusty_modbus_client::ClientConfig::default, |c| c.to_rust());
        let tls_cfg = tls.to_rust();

        let runtime = Runtime::new()
            .map_err(|e| errors::ConnectionError::new_err(format!("runtime error: {e}")))?;

        let (sink, stream): (TlsSink, TlsRecvStream) = py
            .detach(|| runtime.block_on(TlsTransport::connect(addr, &tls_cfg)))
            .map_err(|e| errors::ConnectionError::new_err(format!("TLS error: {e}")))?;
        let client = RustClient::from_transport(sink, stream, cfg);

        Ok(Self {
            inner: InnerClient::Tls(Arc::new(client)),
            runtime,
        })
    }

    // ── lifecycle ───────────────────────────────────────────────────

    /// Whether the client is currently connected.
    #[getter]
    fn is_connected(&self) -> bool {
        match &self.inner {
            InnerClient::Tcp(c) => c.is_connected(),
            InnerClient::Tls(c) => c.is_connected(),
        }
    }

    /// Gracefully shut down the client.
    fn shutdown(&self, py: Python<'_>) {
        py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(c.shutdown()),
            InnerClient::Tls(c) => self.runtime.block_on(c.shutdown()),
        });
    }

    /// Immediately cancel client work without waiting.
    fn abort(&self) {
        match &self.inner {
            InnerClient::Tcp(c) => c.abort(),
            InnerClient::Tls(c) => c.abort(),
        }
    }

    /// Sync context manager — enter.
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// Sync context manager — exit (calls shutdown).
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: &Bound<'_, PyAny>,
        _exc_val: &Bound<'_, PyAny>,
        _exc_tb: &Bound<'_, PyAny>,
    ) -> bool {
        self.shutdown(py);
        false
    }

    // ── register methods ────────────────────────────────────────────

    /// Read holding registers (FC 0x03).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_holding_registers(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Vec<u16>> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime
                    .block_on(c.read_holding_registers(UnitId(unit_id), address, quantity))
            }
            InnerClient::Tls(c) => {
                self.runtime
                    .block_on(c.read_holding_registers(UnitId(unit_id), address, quantity))
            }
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    /// Read input registers (FC 0x04).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_input_registers(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Vec<u16>> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime
                    .block_on(c.read_input_registers(UnitId(unit_id), address, quantity))
            }
            InnerClient::Tls(c) => {
                self.runtime
                    .block_on(c.read_input_registers(UnitId(unit_id), address, quantity))
            }
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    /// Write a single register (FC 0x06).
    #[pyo3(signature = (unit_id, address, value))]
    fn write_single_register(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        value: u16,
    ) -> PyResult<()> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime
                    .block_on(c.write_single_register(UnitId(unit_id), address, value))
            }
            InnerClient::Tls(c) => {
                self.runtime
                    .block_on(c.write_single_register(UnitId(unit_id), address, value))
            }
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    /// Write multiple registers (FC 0x10).
    #[pyo3(signature = (unit_id, address, values))]
    fn write_multiple_registers(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        values: Vec<u16>,
    ) -> PyResult<()> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime
                    .block_on(c.write_multiple_registers(UnitId(unit_id), address, &values))
            }
            InnerClient::Tls(c) => {
                self.runtime
                    .block_on(c.write_multiple_registers(UnitId(unit_id), address, &values))
            }
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    /// Mask write register (FC 0x16).
    #[pyo3(signature = (unit_id, address, and_mask, or_mask))]
    fn mask_write_register(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        and_mask: u16,
        or_mask: u16,
    ) -> PyResult<()> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(c.mask_write_register(
                UnitId(unit_id),
                address,
                and_mask,
                or_mask,
            )),
            InnerClient::Tls(c) => self.runtime.block_on(c.mask_write_register(
                UnitId(unit_id),
                address,
                and_mask,
                or_mask,
            )),
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    /// Read and write multiple registers (FC 0x17).
    #[pyo3(signature = (unit_id, read_address, read_quantity, write_address, write_values))]
    fn read_write_multiple_registers(
        &self,
        py: Python<'_>,
        unit_id: u8,
        read_address: u16,
        read_quantity: u16,
        write_address: u16,
        write_values: Vec<u16>,
    ) -> PyResult<Vec<u16>> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => self.runtime.block_on(c.read_write_multiple_registers(
                UnitId(unit_id),
                read_address,
                read_quantity,
                write_address,
                &write_values,
            )),
            InnerClient::Tls(c) => self.runtime.block_on(c.read_write_multiple_registers(
                UnitId(unit_id),
                read_address,
                read_quantity,
                write_address,
                &write_values,
            )),
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    // ── coil / discrete input methods ───────────────────────────────

    /// Read coils (FC 0x01).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_coils(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Vec<bool>> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime
                    .block_on(c.read_coils(UnitId(unit_id), address, quantity))
            }
            InnerClient::Tls(c) => {
                self.runtime
                    .block_on(c.read_coils(UnitId(unit_id), address, quantity))
            }
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    /// Read discrete inputs (FC 0x02).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_discrete_inputs(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Vec<bool>> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime
                    .block_on(c.read_discrete_inputs(UnitId(unit_id), address, quantity))
            }
            InnerClient::Tls(c) => {
                self.runtime
                    .block_on(c.read_discrete_inputs(UnitId(unit_id), address, quantity))
            }
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    /// Write a single coil (FC 0x05).
    #[pyo3(signature = (unit_id, address, value))]
    fn write_single_coil(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        value: bool,
    ) -> PyResult<()> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime
                    .block_on(c.write_single_coil(UnitId(unit_id), address, value))
            }
            InnerClient::Tls(c) => {
                self.runtime
                    .block_on(c.write_single_coil(UnitId(unit_id), address, value))
            }
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    /// Write multiple coils (FC 0x0F).
    #[pyo3(signature = (unit_id, address, values))]
    fn write_multiple_coils(
        &self,
        py: Python<'_>,
        unit_id: u8,
        address: u16,
        values: Vec<bool>,
    ) -> PyResult<()> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => {
                self.runtime
                    .block_on(c.write_multiple_coils(UnitId(unit_id), address, &values))
            }
            InnerClient::Tls(c) => {
                self.runtime
                    .block_on(c.write_multiple_coils(UnitId(unit_id), address, &values))
            }
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    // ── FIFO ────────────────────────────────────────────────────────

    /// Read FIFO queue (FC 0x18).
    #[pyo3(signature = (unit_id, pointer_address))]
    fn read_fifo_queue(
        &self,
        py: Python<'_>,
        unit_id: u8,
        pointer_address: u16,
    ) -> PyResult<Vec<u16>> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => self
                .runtime
                .block_on(c.read_fifo_queue(UnitId(unit_id), pointer_address)),
            InnerClient::Tls(c) => self
                .runtime
                .block_on(c.read_fifo_queue(UnitId(unit_id), pointer_address)),
        });
        result.map_err(errors::client_error_to_pyerr)
    }

    // ── file record ─────────────────────────────────────────────────

    /// Read file record (FC 0x14). Returns raw response bytes.
    #[pyo3(signature = (unit_id, sub_request_data))]
    fn read_file_record(
        &self,
        py: Python<'_>,
        unit_id: u8,
        sub_request_data: Vec<u8>,
    ) -> PyResult<Vec<u8>> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => self
                .runtime
                .block_on(c.read_file_record(UnitId(unit_id), &sub_request_data)),
            InnerClient::Tls(c) => self
                .runtime
                .block_on(c.read_file_record(UnitId(unit_id), &sub_request_data)),
        });
        result
            .map(|r| r.data.to_vec())
            .map_err(errors::client_error_to_pyerr)
    }

    /// Write file record (FC 0x15). Returns raw response bytes.
    #[pyo3(signature = (unit_id, sub_request_data))]
    fn write_file_record(
        &self,
        py: Python<'_>,
        unit_id: u8,
        sub_request_data: Vec<u8>,
    ) -> PyResult<Vec<u8>> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => self
                .runtime
                .block_on(c.write_file_record(UnitId(unit_id), &sub_request_data)),
            InnerClient::Tls(c) => self
                .runtime
                .block_on(c.write_file_record(UnitId(unit_id), &sub_request_data)),
        });
        result
            .map(|r| r.data.to_vec())
            .map_err(errors::client_error_to_pyerr)
    }

    // ── device identification ───────────────────────────────────────

    /// Read device identification (FC 0x2B / MEI 0x0E).
    #[pyo3(signature = (unit_id))]
    fn read_device_identification(
        &self,
        py: Python<'_>,
        unit_id: u8,
    ) -> PyResult<DeviceIdentification> {
        let result = py.detach(|| match &self.inner {
            InnerClient::Tcp(c) => self
                .runtime
                .block_on(c.read_device_identification(UnitId(unit_id))),
            InnerClient::Tls(c) => self
                .runtime
                .block_on(c.read_device_identification(UnitId(unit_id))),
        });
        result
            .map(DeviceIdentification::from)
            .map_err(errors::client_error_to_pyerr)
    }
}
