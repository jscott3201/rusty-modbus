//! Integration tests for RTU-over-TCP transport.
//!
//! CI-friendly — no serial hardware required.

use bytes::Bytes;
use rusty_modbus_codec::{ResponsePdu, decode_response};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_rtu::config::RtuConfig;
use rusty_modbus_rtu::rtu_tcp::RtuOverTcpTransport;
use rusty_modbus_tcp::config::TcpConfig;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use std::net::SocketAddr;
use std::time::Duration;

/// Start an RTU-over-TCP echo server on an ephemeral port.
async fn start_rtu_tcp_echo_server() -> SocketAddr {
    use futures_util::{SinkExt, StreamExt};
    use rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;
    use tokio::net::TcpListener;
    use tokio_util::codec::Framed;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(stream, RtuOverTcpCodec);
        // Echo back up to 10 frames.
        for _ in 0..10 {
            match framed.next().await {
                Some(Ok(frame)) => {
                    if framed.send(frame).await.is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });

    addr
}

/// Start an RTU-over-TCP server that responds with a proper Modbus response.
async fn start_rtu_tcp_modbus_server() -> SocketAddr {
    use futures_util::{SinkExt, StreamExt};
    use rusty_modbus_frame::rtu_tcp::RtuOverTcpCodec;
    use tokio::net::TcpListener;
    use tokio_util::codec::Framed;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(stream, RtuOverTcpCodec);

        if let Some(Ok(req_frame)) = framed.next().await {
            // Build a ReadHoldingRegisters response: FC 0x03, byte_count=4, regs=[0xABCD, 0x1234]
            let response_pdu = Bytes::from_static(&[0x03, 0x04, 0xAB, 0xCD, 0x12, 0x34]);
            let resp_frame = Frame {
                header: FrameHeader::Rtu {
                    unit_id: req_frame.unit_id(),
                },
                pdu: response_pdu,
            };
            let _ = framed.send(resp_frame).await;
        }
    });

    addr
}

fn make_rtu_frame(unit_id: u8, pdu: &[u8]) -> Frame {
    Frame {
        header: FrameHeader::Rtu { unit_id },
        pdu: Bytes::copy_from_slice(pdu),
    }
}

#[tokio::test]
async fn rtu_over_tcp_echo_roundtrip() {
    let addr = start_rtu_tcp_echo_server().await;
    let config = TcpConfig {
        connect_timeout: Duration::from_secs(2),
        ..TcpConfig::default()
    };

    let (mut sink, mut stream) = RtuOverTcpTransport::connect(addr, config).await.unwrap();

    // Send a ReadHoldingRegisters request.
    let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
    let frame = make_rtu_frame(0x01, &pdu);
    sink.send(frame).await.unwrap();

    // Receive the echo.
    let response = stream.recv().await.unwrap();
    assert_eq!(response.unit_id(), 0x01);
    assert_eq!(&response.pdu[..], &pdu);
}

#[tokio::test]
async fn rtu_over_tcp_full_modbus_decode() {
    let addr = start_rtu_tcp_modbus_server().await;
    let config = TcpConfig {
        connect_timeout: Duration::from_secs(2),
        ..TcpConfig::default()
    };

    let (mut sink, mut stream) = RtuOverTcpTransport::connect(addr, config).await.unwrap();

    // Send request: FC 0x03, start=0x0000, qty=0x0002
    let request_pdu = [0x03, 0x00, 0x00, 0x00, 0x02];
    sink.send(make_rtu_frame(0x01, &request_pdu)).await.unwrap();

    // Receive and decode response.
    let response = stream.recv().await.unwrap();
    let decoded = decode_response(&response.pdu).unwrap();
    match decoded {
        ResponsePdu::ReadHoldingRegisters(rhr) => {
            assert_eq!(rhr.count(), 2);
            assert_eq!(rhr.register(0), 0xABCD);
            assert_eq!(rhr.register(1), 0x1234);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn rtu_over_tcp_disconnect_detection() {
    let addr = start_rtu_tcp_echo_server().await;
    let config = TcpConfig {
        connect_timeout: Duration::from_secs(2),
        read_timeout: Some(Duration::from_secs(1)),
        ..TcpConfig::default()
    };

    let (_sink, mut stream) = RtuOverTcpTransport::connect(addr, config).await.unwrap();

    // Don't send anything — server will close after accept.
    // Give server time to close.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // recv should return Timeout or Disconnected.
    let result = stream.recv().await;
    assert!(result.is_err(), "expected error, got {result:?}");
}

#[tokio::test]
async fn rtu_over_tcp_concurrent_send_recv() {
    let addr = start_rtu_tcp_echo_server().await;
    let config = TcpConfig {
        connect_timeout: Duration::from_secs(2),
        ..TcpConfig::default()
    };

    let (mut sink, mut stream) = RtuOverTcpTransport::connect(addr, config).await.unwrap();

    // Send 3 frames.
    let writer = tokio::spawn(async move {
        for i in 1..=3u8 {
            let pdu = [0x03, 0x00, i, 0x00, 0x01];
            sink.send(make_rtu_frame(i, &pdu)).await.unwrap();
        }
    });

    // Receive 3 echoes.
    let mut received = Vec::new();
    for _ in 0..3 {
        let frame = stream.recv().await.unwrap();
        received.push(frame);
    }

    writer.await.unwrap();
    assert_eq!(received.len(), 3);
    assert_eq!(received[0].unit_id(), 1);
    assert_eq!(received[1].unit_id(), 2);
    assert_eq!(received[2].unit_id(), 3);
}

#[test]
fn interframe_delay_calculation() {
    let config = RtuConfig::default();
    // At 9600 baud: 3.5 * 11 / 9600 ≈ 4.01ms
    let delay = config.interframe_delay();
    assert!(delay > Duration::from_micros(3500));
    assert!(delay < Duration::from_micros(5000));

    // At 19200: 3.5 * 11 / 19200 ≈ 2.0ms
    let fast = RtuConfig {
        baud_rate: 19200,
        ..RtuConfig::default()
    };
    let delay = fast.interframe_delay();
    assert!(delay > Duration::from_micros(1500));
    assert!(delay < Duration::from_micros(2500));

    // Above 19200: fixed 1.75ms
    let faster = RtuConfig {
        baud_rate: 115_200,
        ..RtuConfig::default()
    };
    assert_eq!(faster.interframe_delay(), Duration::from_micros(1750));
}
