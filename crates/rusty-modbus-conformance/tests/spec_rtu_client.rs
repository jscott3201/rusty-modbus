//! Spec: RTU over the pipelined `ModbusClient`.
//!
//! RTU frames carry no transaction ID, so the client must be single-in-flight
//! and match each response to the one outstanding request. Before the fix the
//! reader dispatched every RTU response to `TransactionId(0)`, which never
//! matched a registered transaction, so every request timed out.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;
use rusty_modbus_rtu::RtuOverTcpTransport;
use rusty_modbus_tcp::config::TcpConfig;
use rusty_modbus_types::UnitId;
use tokio::net::TcpListener;
use tokio_util::codec::Framed;

/// An RTU-over-TCP device that answers FC 0x03 with fixed registers, echoing the
/// request's unit ID back on the RTU response frame.
async fn start_rtu_tcp_device(registers: Vec<u16>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let regs = registers.clone();
            tokio::spawn(async move {
                let mut framed: Framed<_, RtuOverTcpCodec> = Framed::new(stream, RtuOverTcpCodec);
                while let Some(Ok(frame)) = framed.next().await {
                    let unit_id = frame.unit_id();
                    let fc = frame.pdu[0];
                    let resp_pdu = if fc == 0x03 {
                        let mut pdu = vec![0x03, (regs.len() * 2) as u8];
                        for &r in &regs {
                            pdu.extend_from_slice(&r.to_be_bytes());
                        }
                        pdu
                    } else {
                        vec![fc | 0x80, 0x01]
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

    tokio::time::sleep(Duration::from_millis(10)).await;
    addr
}

async fn start_rtu_tcp_device_with_foreign_response() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut framed: Framed<_, RtuOverTcpCodec> = Framed::new(stream, RtuOverTcpCodec);
        let request = framed.next().await.unwrap().unwrap();
        let unit_id = request.unit_id();

        framed
            .send(Frame {
                header: FrameHeader::Rtu {
                    unit_id: unit_id + 1,
                },
                pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x11]),
            })
            .await
            .unwrap();
        framed
            .send(Frame {
                header: FrameHeader::Rtu { unit_id },
                pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
            })
            .await
            .unwrap();
    });

    addr
}

#[tokio::test]
async fn rtu_client_matches_sequential_requests() {
    let addr = start_rtu_tcp_device(vec![0x1234, 0x5678]).await;
    let (sink, stream) = RtuOverTcpTransport::connect(addr, TcpConfig::default())
        .await
        .unwrap();
    let config = ClientConfig {
        timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_rtu_transport(sink, stream, config);

    // Each sequential request must be matched to its response. Before the fix
    // these all timed out (responses went to TransactionId(0)).
    for _ in 0..5 {
        let regs = client
            .read_holding_registers(UnitId(1), 0, 2)
            .await
            .unwrap();
        assert_eq!(regs, vec![0x1234, 0x5678]);
    }

    client.shutdown().await;
}

#[tokio::test]
async fn rtu_client_forces_single_in_flight() {
    // Even when the caller requests a high concurrency, an RTU client must clamp
    // to one in-flight request (half-duplex). We can't observe max_in_flight
    // directly, but a burst of concurrent calls must all still succeed (they are
    // serialized by the single permit and matched FIFO).
    let addr = start_rtu_tcp_device(vec![0x00AA]).await;
    let (sink, stream) = RtuOverTcpTransport::connect(addr, TcpConfig::default())
        .await
        .unwrap();
    let config = ClientConfig {
        timeout: Duration::from_secs(2),
        max_in_flight: 16,
        ..ClientConfig::default()
    };
    let client = std::sync::Arc::new(ModbusClient::from_rtu_transport(sink, stream, config));

    let mut handles = Vec::new();
    for _ in 0..8 {
        let c = std::sync::Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.read_holding_registers(UnitId(1), 0, 1).await
        }));
    }
    for h in handles {
        assert_eq!(h.await.unwrap().unwrap(), vec![0x00AA]);
    }
}

#[tokio::test]
async fn rtu_client_ignores_other_unit_before_matching_response() {
    let addr = start_rtu_tcp_device_with_foreign_response().await;
    let (sink, stream) = RtuOverTcpTransport::connect(addr, TcpConfig::default())
        .await
        .unwrap();
    let config = ClientConfig {
        timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    };
    let client = ModbusClient::from_rtu_transport(sink, stream, config);

    let registers = client
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap();
    assert_eq!(registers, vec![0x002A]);
}
