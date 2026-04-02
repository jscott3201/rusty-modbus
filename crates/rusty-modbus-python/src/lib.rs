//! Python bindings for the rusty-modbus client.

use pyo3::prelude::*;

mod client;
mod config;
mod errors;
mod sync_client;
mod types;

use std::net::SocketAddr;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::MbapHeader;

/// Start an embedded Modbus test server on a random port (synchronous).
///
/// Returns the ``host:port`` address string. The server runs on a dedicated
/// background tokio runtime that lives for the duration of the process.
#[pyfunction]
fn _start_test_server() -> PyResult<String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("runtime: {e}")))?;

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let listener = runtime
        .block_on(TcpServerListener::bind(addr, TcpServerConfig::default()))
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("bind failed: {e}")))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("local_addr: {e}")))?;

    // Spawn the accept loop and leak the runtime so it lives for the process.
    std::thread::spawn(move || {
        runtime.block_on(async move {
            loop {
                let accept_result = listener.accept().await;
                let (mut sink, mut stream, _peer) = match accept_result {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                tokio::spawn(async move {
                    loop {
                        let frame = match stream.recv().await {
                            Ok(f) => f,
                            Err(_) => break,
                        };

                        let pdu = &frame.pdu[..];
                        if pdu.is_empty() {
                            break;
                        }

                        let fc = pdu[0];
                        let (txn_id, unit_id) = match frame.header {
                            FrameHeader::Mbap(h) => (h.transaction_id.get(), h.unit_id),
                            FrameHeader::Rtu { unit_id } => (0, unit_id),
                        };

                        let resp_pdu: Vec<u8> = match fc {
                            // FC 0x03 ReadHoldingRegisters, FC 0x04 ReadInputRegisters
                            0x03 | 0x04 => vec![fc, 0x04, 0x00, 0x01, 0x00, 0x02],
                            // FC 0x06 WriteSingleRegister — echo request PDU
                            0x06 => pdu.to_vec(),
                            // FC 0x10 WriteMultipleRegisters — echo [fc, addr_hi, addr_lo, qty_hi, qty_lo]
                            0x10 => {
                                if pdu.len() >= 5 {
                                    vec![fc, pdu[1], pdu[2], pdu[3], pdu[4]]
                                } else {
                                    vec![fc | 0x80, 0x01]
                                }
                            }
                            // FC 0x16 MaskWriteRegister — echo request PDU
                            0x16 => pdu.to_vec(),
                            // FC 0x17 ReadWriteMultipleRegisters — return 2 registers
                            0x17 => vec![fc, 0x04, 0x00, 0x01, 0x00, 0x02],
                            // FC 0x01 ReadCoils, FC 0x02 ReadDiscreteInputs
                            0x01 | 0x02 => vec![fc, 0x01, 0x05],
                            // FC 0x05 WriteSingleCoil — echo request PDU
                            0x05 => pdu.to_vec(),
                            // FC 0x0F WriteMultipleCoils — echo [fc, addr_hi, addr_lo, qty_hi, qty_lo]
                            0x0F => {
                                if pdu.len() >= 5 {
                                    vec![fc, pdu[1], pdu[2], pdu[3], pdu[4]]
                                } else {
                                    vec![fc | 0x80, 0x01]
                                }
                            }
                            // FC 0x18 ReadFifoQueue
                            // byte_count=6 (2 for fifo_count + 4 for 2 registers),
                            // fifo_count=2, values=[0x000A, 0x000B]
                            0x18 => vec![
                                fc, 0x00, 0x06, 0x00, 0x02, 0x00, 0x0A, 0x00, 0x0B,
                            ],
                            // Unknown — IllegalFunction
                            _ => vec![fc | 0x80, 0x01],
                        };

                        let header =
                            MbapHeader::new(txn_id, unit_id, resp_pdu.len() as u16);
                        let resp_frame = Frame {
                            header: FrameHeader::Mbap(header),
                            pdu: Bytes::from(resp_pdu),
                        };

                        if sink.send(resp_frame).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
    });

    Ok(local_addr.to_string())
}

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
    m.add_function(wrap_pyfunction!(_start_test_server, m)?)?;
    Ok(())
}
