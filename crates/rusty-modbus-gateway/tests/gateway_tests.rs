//! Integration tests for the Modbus gateway.
//!
//! Uses RTU-over-TCP backends to simulate serial devices (CI-friendly).

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{ClientConfig, ClientError, ModbusClient};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;
use rusty_modbus_gateway::{GatewayConfig, ModbusGateway, RouteEntry};
use rusty_modbus_types::{ExceptionCode, UnitId};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_util::codec::Framed;

/// Start an RTU-over-TCP echo device that responds to FC 0x03 with fixed registers.
async fn start_rtu_device(registers: Vec<u16>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let regs = registers.clone();
            tokio::spawn(async move {
                let mut framed: Framed<_, RtuOverTcpCodec> = Framed::new(stream, RtuOverTcpCodec);
                while let Some(Ok(frame)) = framed.next().await {
                    let fc = frame.pdu[0];
                    let unit_id = frame.unit_id();

                    let resp_pdu = if fc == 0x03 {
                        let byte_count = (regs.len() * 2) as u8;
                        let mut pdu = vec![0x03, byte_count];
                        for &r in &regs {
                            pdu.extend_from_slice(&r.to_be_bytes());
                        }
                        pdu
                    } else if fc == 0x06 {
                        // Echo write single register.
                        frame.pdu.to_vec()
                    } else {
                        vec![fc | 0x80, 0x01] // IllegalFunction
                    };

                    let resp = Frame {
                        header: FrameHeader::Rtu { unit_id },
                        pdu: Bytes::from(resp_pdu),
                    };
                    if framed.send(resp).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    // Give listener time to start.
    tokio::time::sleep(Duration::from_millis(10)).await;
    addr
}

fn client_config() -> ClientConfig {
    ClientConfig {
        timeout: Duration::from_secs(3),
        ..ClientConfig::default()
    }
}

#[tokio::test]
async fn gateway_routes_request_to_backend() {
    let device_addr = start_rtu_device(vec![0xAAAA, 0xBBBB]).await;

    let gw_config = GatewayConfig {
        tcp_listen: "127.0.0.1:0".parse().unwrap(),
        routes: vec![RouteEntry {
            unit_id_range: 1..=10,
            backend_addr: device_addr,
        }],
        serial_timeout: Duration::from_secs(2),
        ..GatewayConfig::default()
    };

    let gateway = ModbusGateway::start(gw_config).await.unwrap();
    let gw_addr = gateway.local_addr();

    let client = ModbusClient::connect(gw_addr, client_config())
        .await
        .unwrap();
    let regs = client
        .read_holding_registers(UnitId(1), 0, 2)
        .await
        .unwrap();
    assert_eq!(regs, vec![0xAAAA, 0xBBBB]);
}

#[tokio::test]
async fn gateway_returns_path_unavailable_for_unrouted_unit_id() {
    let device_addr = start_rtu_device(vec![0x1234]).await;

    let gw_config = GatewayConfig {
        tcp_listen: "127.0.0.1:0".parse().unwrap(),
        routes: vec![RouteEntry {
            unit_id_range: 1..=5,
            backend_addr: device_addr,
        }],
        serial_timeout: Duration::from_secs(1),
        ..GatewayConfig::default()
    };

    let gateway = ModbusGateway::start(gw_config).await.unwrap();
    let client = ModbusClient::connect(gateway.local_addr(), client_config())
        .await
        .unwrap();

    // Unit ID 20 is not in any route.
    let result = client.read_holding_registers(UnitId(20), 0, 1).await;
    match result {
        Err(ClientError::Exception(exc)) => {
            assert_eq!(exc.exception_code, ExceptionCode::GatewayPathUnavailable);
        }
        other => panic!("expected GatewayPathUnavailable, got {other:?}"),
    }
}

#[tokio::test]
async fn gateway_returns_timeout_for_unresponsive_device() {
    // Bind a listener but never respond.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        // Accept but never send any response.
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut framed = Framed::new(stream, RtuOverTcpCodec);
                // Read one frame then drop (simulate unresponsive device).
                let _ = framed.next().await;
                tokio::time::sleep(Duration::from_secs(10)).await;
            });
        }
    });

    let gw_config = GatewayConfig {
        tcp_listen: "127.0.0.1:0".parse().unwrap(),
        routes: vec![RouteEntry {
            unit_id_range: 1..=10,
            backend_addr: dead_addr,
        }],
        serial_timeout: Duration::from_millis(500),
        ..GatewayConfig::default()
    };

    let gateway = ModbusGateway::start(gw_config).await.unwrap();
    let client = ModbusClient::connect(
        gateway.local_addr(),
        ClientConfig {
            timeout: Duration::from_secs(5),
            ..ClientConfig::default()
        },
    )
    .await
    .unwrap();

    let result = client.read_holding_registers(UnitId(1), 0, 1).await;
    match result {
        Err(ClientError::Exception(exc)) => {
            assert_eq!(
                exc.exception_code,
                ExceptionCode::GatewayTargetDeviceFailedToRespond
            );
        }
        other => panic!("expected GatewayTargetDeviceFailedToRespond, got {other:?}"),
    }
}

#[tokio::test]
async fn gateway_multiple_routes() {
    let dev1 = start_rtu_device(vec![0x0001]).await;
    let dev2 = start_rtu_device(vec![0x0002]).await;

    let gw_config = GatewayConfig {
        tcp_listen: "127.0.0.1:0".parse().unwrap(),
        routes: vec![
            RouteEntry {
                unit_id_range: 1..=10,
                backend_addr: dev1,
            },
            RouteEntry {
                unit_id_range: 11..=20,
                backend_addr: dev2,
            },
        ],
        serial_timeout: Duration::from_secs(2),
        ..GatewayConfig::default()
    };

    let gateway = ModbusGateway::start(gw_config).await.unwrap();
    let client = ModbusClient::connect(gateway.local_addr(), client_config())
        .await
        .unwrap();

    let r1 = client
        .read_holding_registers(UnitId(5), 0, 1)
        .await
        .unwrap();
    let r2 = client
        .read_holding_registers(UnitId(15), 0, 1)
        .await
        .unwrap();

    assert_eq!(r1, vec![0x0001]);
    assert_eq!(r2, vec![0x0002]);
}

/// Start an RTU device that answers every request with a 1-register body but
/// stamps the response with a chosen unit_id / function code — used to simulate
/// shared-bus cross-talk (wrong slave) or a desynchronized response (wrong FC).
async fn start_misbehaving_rtu_device(resp_unit: u8, resp_fc: u8) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut framed: Framed<_, RtuOverTcpCodec> = Framed::new(stream, RtuOverTcpCodec);
                while let Some(Ok(_frame)) = framed.next().await {
                    let resp = Frame {
                        header: FrameHeader::Rtu { unit_id: resp_unit },
                        pdu: Bytes::from(vec![resp_fc, 0x02, 0x12, 0x34]),
                    };
                    if framed.send(resp).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    tokio::time::sleep(Duration::from_millis(10)).await;
    addr
}

#[tokio::test]
async fn gateway_rejects_response_from_wrong_unit_id() {
    // Client addresses unit 1, but the device stamps its reply with unit 7
    // (cross-talk on a shared bus). The gateway must NOT relay it as unit 1.
    let device_addr = start_misbehaving_rtu_device(7, 0x03).await;

    let gw_config = GatewayConfig {
        tcp_listen: "127.0.0.1:0".parse().unwrap(),
        routes: vec![RouteEntry {
            unit_id_range: 1..=10,
            backend_addr: device_addr,
        }],
        serial_timeout: Duration::from_secs(2),
        ..GatewayConfig::default()
    };

    let gateway = ModbusGateway::start(gw_config).await.unwrap();
    let client = ModbusClient::connect(gateway.local_addr(), client_config())
        .await
        .unwrap();

    let result = client.read_holding_registers(UnitId(1), 0, 1).await;
    match result {
        Err(ClientError::Exception(exc)) => assert_eq!(
            exc.exception_code,
            ExceptionCode::GatewayTargetDeviceFailedToRespond
        ),
        other => panic!("expected GatewayTargetDeviceFailedToRespond, got {other:?}"),
    }
}

#[tokio::test]
async fn gateway_rejects_response_with_wrong_function_code() {
    // Correct unit, but the device answers an FC03 request with an FC04 body.
    let device_addr = start_misbehaving_rtu_device(1, 0x04).await;

    let gw_config = GatewayConfig {
        tcp_listen: "127.0.0.1:0".parse().unwrap(),
        routes: vec![RouteEntry {
            unit_id_range: 1..=10,
            backend_addr: device_addr,
        }],
        serial_timeout: Duration::from_secs(2),
        ..GatewayConfig::default()
    };

    let gateway = ModbusGateway::start(gw_config).await.unwrap();
    let client = ModbusClient::connect(gateway.local_addr(), client_config())
        .await
        .unwrap();

    let result = client.read_holding_registers(UnitId(1), 0, 1).await;
    match result {
        Err(ClientError::Exception(exc)) => assert_eq!(
            exc.exception_code,
            ExceptionCode::GatewayTargetDeviceFailedToRespond
        ),
        other => panic!("expected GatewayTargetDeviceFailedToRespond, got {other:?}"),
    }
}
