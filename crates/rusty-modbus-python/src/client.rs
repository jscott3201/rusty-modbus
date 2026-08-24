//! Async `ModbusClient` — Python awaitable wrapper around the Rust client.

use std::net::SocketAddr;
use std::sync::Arc;

use pyo3::prelude::*;
use rusty_modbus_client::ModbusClient as RustClient;
use rusty_modbus_tcp::TcpSink;
use rusty_modbus_tls::{TlsRecvStream, TlsSink, TlsTransport};
use rusty_modbus_types::UnitId;

use crate::config::{ClientConfig, TlsConfig};
use crate::errors;
use crate::types::DeviceIdentification;

/// Transport-agnostic handle: either a TCP or TLS inner client.
#[derive(Clone)]
enum InnerClient {
    Tcp(Arc<RustClient<TcpSink>>),
    Tls(Arc<RustClient<TlsSink>>),
}

/// Async Modbus client for use with Python ``asyncio``.
///
/// Supports both TCP and TLS transports, the full set of Modbus data
/// methods, and the async context-manager protocol (``async with``).
#[pyclass(module = "rusty_modbus")]
pub struct ModbusClient {
    inner: InnerClient,
}

#[pymethods]
impl ModbusClient {
    // ── construction ────────────────────────────────────────────────

    /// Connect to a Modbus/TCP server.
    #[staticmethod]
    #[pyo3(signature = (address, config=None))]
    fn connect<'py>(
        py: Python<'py>,
        address: &str,
        config: Option<ClientConfig>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let addr: SocketAddr = address
            .parse()
            .map_err(|e| errors::ConnectionError::new_err(format!("invalid address: {e}")))?;
        let cfg = config.map_or_else(rusty_modbus_client::ClientConfig::default, |c| c.to_rust());

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let client = RustClient::connect(addr, cfg)
                .await
                .map_err(errors::client_error_to_pyerr)?;
            Ok(ModbusClient {
                inner: InnerClient::Tcp(Arc::new(client)),
            })
        })
    }

    /// Connect to a Modbus/TCP Security (TLS) server.
    #[staticmethod]
    #[pyo3(signature = (address, tls, config=None))]
    fn connect_tls<'py>(
        py: Python<'py>,
        address: &str,
        tls: TlsConfig,
        config: Option<ClientConfig>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let addr: SocketAddr = address
            .parse()
            .map_err(|e| errors::ConnectionError::new_err(format!("invalid address: {e}")))?;
        let cfg = config.map_or_else(rusty_modbus_client::ClientConfig::default, |c| c.to_rust());
        let tls_cfg = tls.to_rust();

        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let (sink, stream): (TlsSink, TlsRecvStream) = TlsTransport::connect(addr, &tls_cfg)
                .await
                .map_err(|e| errors::ConnectionError::new_err(format!("TLS error: {e}")))?;
            let client = RustClient::from_transport(sink, stream, cfg);
            Ok(ModbusClient {
                inner: InnerClient::Tls(Arc::new(client)),
            })
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

    /// Immediately cancel client work without waiting.
    fn abort(&self) {
        match &self.inner {
            InnerClient::Tcp(c) => c.abort(),
            InnerClient::Tls(c) => c.abort(),
        }
    }

    /// Async context manager — enter.
    fn __aenter__(slf: Py<Self>, py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf) })
    }

    /// Async context manager — exit (calls shutdown).
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: &Bound<'py, PyAny>,
        _exc_val: &Bound<'py, PyAny>,
        _exc_tb: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            match &inner {
                InnerClient::Tcp(c) => c.shutdown().await,
                InnerClient::Tls(c) => c.shutdown().await,
            }
            Ok(false)
        })
    }

    // ── register methods ────────────────────────────────────────────

    /// Read holding registers (FC 0x03).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_holding_registers<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.read_holding_registers(UnitId(unit_id), address, quantity)
                        .await
                }
                InnerClient::Tls(c) => {
                    c.read_holding_registers(UnitId(unit_id), address, quantity)
                        .await
                }
            };
            result.map_err(errors::client_error_to_pyerr)
        })
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.read_input_registers(UnitId(unit_id), address, quantity)
                        .await
                }
                InnerClient::Tls(c) => {
                    c.read_input_registers(UnitId(unit_id), address, quantity)
                        .await
                }
            };
            result.map_err(errors::client_error_to_pyerr)
        })
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.write_single_register(UnitId(unit_id), address, value)
                        .await
                }
                InnerClient::Tls(c) => {
                    c.write_single_register(UnitId(unit_id), address, value)
                        .await
                }
            };
            result.map_err(errors::client_error_to_pyerr)
        })
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.write_multiple_registers(UnitId(unit_id), address, &values)
                        .await
                }
                InnerClient::Tls(c) => {
                    c.write_multiple_registers(UnitId(unit_id), address, &values)
                        .await
                }
            };
            result.map_err(errors::client_error_to_pyerr)
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.mask_write_register(UnitId(unit_id), address, and_mask, or_mask)
                        .await
                }
                InnerClient::Tls(c) => {
                    c.mask_write_register(UnitId(unit_id), address, and_mask, or_mask)
                        .await
                }
            };
            result.map_err(errors::client_error_to_pyerr)
        })
    }

    /// Read and write multiple registers (FC 0x17).
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.read_write_multiple_registers(
                        UnitId(unit_id),
                        read_address,
                        read_quantity,
                        write_address,
                        &write_values,
                    )
                    .await
                }
                InnerClient::Tls(c) => {
                    c.read_write_multiple_registers(
                        UnitId(unit_id),
                        read_address,
                        read_quantity,
                        write_address,
                        &write_values,
                    )
                    .await
                }
            };
            result.map_err(errors::client_error_to_pyerr)
        })
    }

    // ── coil / discrete input methods ───────────────────────────────

    /// Read coils (FC 0x01).
    #[pyo3(signature = (unit_id, address, quantity))]
    fn read_coils<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        address: u16,
        quantity: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.read_coils(UnitId(unit_id), address, quantity).await,
                InnerClient::Tls(c) => c.read_coils(UnitId(unit_id), address, quantity).await,
            };
            result.map_err(errors::client_error_to_pyerr)
        })
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.read_discrete_inputs(UnitId(unit_id), address, quantity)
                        .await
                }
                InnerClient::Tls(c) => {
                    c.read_discrete_inputs(UnitId(unit_id), address, quantity)
                        .await
                }
            };
            result.map_err(errors::client_error_to_pyerr)
        })
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.write_single_coil(UnitId(unit_id), address, value).await,
                InnerClient::Tls(c) => c.write_single_coil(UnitId(unit_id), address, value).await,
            };
            result.map_err(errors::client_error_to_pyerr)
        })
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
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.write_multiple_coils(UnitId(unit_id), address, &values)
                        .await
                }
                InnerClient::Tls(c) => {
                    c.write_multiple_coils(UnitId(unit_id), address, &values)
                        .await
                }
            };
            result.map_err(errors::client_error_to_pyerr)
        })
    }

    // ── FIFO ────────────────────────────────────────────────────────

    /// Read FIFO queue (FC 0x18).
    #[pyo3(signature = (unit_id, pointer_address))]
    fn read_fifo_queue<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        pointer_address: u16,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.read_fifo_queue(UnitId(unit_id), pointer_address).await,
                InnerClient::Tls(c) => c.read_fifo_queue(UnitId(unit_id), pointer_address).await,
            };
            result.map_err(errors::client_error_to_pyerr)
        })
    }

    // ── file record ─────────────────────────────────────────────────

    /// Read file record (FC 0x14). Returns raw response bytes.
    #[pyo3(signature = (unit_id, sub_request_data))]
    fn read_file_record<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        sub_request_data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.read_file_record(UnitId(unit_id), &sub_request_data).await,
                InnerClient::Tls(c) => c.read_file_record(UnitId(unit_id), &sub_request_data).await,
            };
            result
                .map(|r| r.data.to_vec())
                .map_err(errors::client_error_to_pyerr)
        })
    }

    /// Write file record (FC 0x15). Returns raw response bytes.
    #[pyo3(signature = (unit_id, sub_request_data))]
    fn write_file_record<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
        sub_request_data: Vec<u8>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => {
                    c.write_file_record(UnitId(unit_id), &sub_request_data)
                        .await
                }
                InnerClient::Tls(c) => {
                    c.write_file_record(UnitId(unit_id), &sub_request_data)
                        .await
                }
            };
            result
                .map(|r| r.data.to_vec())
                .map_err(errors::client_error_to_pyerr)
        })
    }

    // ── device identification ───────────────────────────────────────

    /// Read device identification (FC 0x2B / MEI 0x0E).
    #[pyo3(signature = (unit_id))]
    fn read_device_identification<'py>(
        &self,
        py: Python<'py>,
        unit_id: u8,
    ) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = match &inner {
                InnerClient::Tcp(c) => c.read_device_identification(UnitId(unit_id)).await,
                InnerClient::Tls(c) => c.read_device_identification(UnitId(unit_id)).await,
            };
            result
                .map(DeviceIdentification::from)
                .map_err(errors::client_error_to_pyerr)
        })
    }
}
