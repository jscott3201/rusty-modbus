//! Integration tests for ModbusClient.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient, RetryConfig};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::config::TcpServerConfig;
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{MbapHeader, UnitId};

/// Start a test server that responds to ReadHoldingRegisters with [0x1234, 0x5678].
async fn start_register_server() -> SocketAddr {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            tokio::spawn(async move {
                while let Ok(req_frame) = stream.recv().await {
                    let txn_id = match req_frame.header {
                        FrameHeader::Mbap(h) => h.transaction_id.get(),
                        FrameHeader::Rtu { .. } => 0,
                    };
                    let unit_id = req_frame.unit_id();
                    let fc = req_frame.pdu[0];

                    let resp_pdu: Vec<u8> = if fc == 0x03 || fc == 0x04 {
                        // ReadHoldingRegisters/ReadInputRegisters response.
                        vec![fc, 0x04, 0x12, 0x34, 0x56, 0x78]
                    } else if fc == 0x06 {
                        // WriteSingleRegister echo.
                        req_frame.pdu.to_vec()
                    } else if fc == 0x10 {
                        // WriteMultipleRegisters response: echo first 4 bytes (FC, addr, qty).
                        let mut resp = vec![fc];
                        resp.extend_from_slice(&req_frame.pdu[1..5]);
                        resp
                    } else if fc == 0x05 {
                        // WriteSingleCoil echo.
                        req_frame.pdu.to_vec()
                    } else {
                        // Unknown FC — return exception.
                        vec![fc | 0x80, 0x01]
                    };

                    let header = MbapHeader::new(txn_id, unit_id, resp_pdu.len() as u16);
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

    addr
}

/// Start a server that returns exception on first request, then succeeds.
async fn start_busy_then_ok_server() -> SocketAddr {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let call_count = Arc::new(AtomicU32::new(0));

    tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            let count = Arc::clone(&call_count);
            tokio::spawn(async move {
                while let Ok(req_frame) = stream.recv().await {
                    let txn_id = match req_frame.header {
                        FrameHeader::Mbap(h) => h.transaction_id.get(),
                        FrameHeader::Rtu { .. } => 0,
                    };
                    let n = count.fetch_add(1, Ordering::Relaxed);

                    let resp_pdu = if n == 0 {
                        // First call: ServerDeviceBusy exception.
                        vec![0x83, 0x06]
                    } else {
                        // Subsequent: success.
                        vec![0x03, 0x02, 0x00, 0x42]
                    };

                    let header =
                        MbapHeader::new(txn_id, req_frame.unit_id(), resp_pdu.len() as u16);
                    let resp = Frame {
                        header: FrameHeader::Mbap(header),
                        pdu: Bytes::from(resp_pdu),
                    };
                    if sink.send(resp).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    addr
}

fn default_config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    }
}

#[tokio::test]
async fn read_holding_registers() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let regs = client
        .read_holding_registers(UnitId(0xFF), 0, 2)
        .await
        .unwrap();

    assert_eq!(regs, vec![0x1234, 0x5678]);
}

#[tokio::test]
async fn read_input_registers() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let regs = client
        .read_input_registers(UnitId(0xFF), 0, 2)
        .await
        .unwrap();

    assert_eq!(regs, vec![0x1234, 0x5678]);
}

#[tokio::test]
async fn write_single_register() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    client
        .write_single_register(UnitId(0xFF), 0x0001, 0xABCD)
        .await
        .unwrap();
}

#[tokio::test]
async fn write_multiple_registers() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    client
        .write_multiple_registers(UnitId(0xFF), 0x0001, &[0x0001, 0x0002])
        .await
        .unwrap();
}

#[tokio::test]
async fn write_single_coil() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    client
        .write_single_coil(UnitId(0xFF), 0x0000, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn broadcast_read_rejected() {
    let addr = start_register_server().await;
    let client = ModbusClient::connect(addr, default_config()).await.unwrap();

    let result = client.read_holding_registers(UnitId(0x00), 0, 1).await;
    assert!(matches!(result, Err(ClientError::BroadcastReadNotAllowed)));
}

#[tokio::test]
async fn pipelining_concurrent_requests() {
    let addr = start_register_server().await;
    let client = std::sync::Arc::new(ModbusClient::connect(addr, default_config()).await.unwrap());

    let mut handles = Vec::new();
    for _ in 0..4 {
        let c = std::sync::Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.read_holding_registers(UnitId(0xFF), 0, 2).await
        }));
    }

    for h in handles {
        let result = h.await.unwrap().unwrap();
        assert_eq!(result, vec![0x1234, 0x5678]);
    }
}

#[tokio::test]
async fn retry_on_server_device_busy() {
    let addr = start_busy_then_ok_server().await;
    let config = ClientConfig {
        timeout: Duration::from_secs(2),
        retry: RetryConfig {
            max_retries: 3,
            retry_delay: Duration::from_millis(50),
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    };

    let client = ModbusClient::connect(addr, config).await.unwrap();
    let regs = client
        .read_holding_registers(UnitId(0xFF), 0, 1)
        .await
        .unwrap();

    assert_eq!(regs, vec![0x0042]);
}

#[tokio::test]
async fn shutdown_cancels_pending() {
    let addr = start_register_server().await;
    let client = std::sync::Arc::new(ModbusClient::connect(addr, default_config()).await.unwrap());

    client.shutdown().await;

    let result = client.read_holding_registers(UnitId(0xFF), 0, 1).await;
    assert!(matches!(result, Err(ClientError::NotConnected)));
}
