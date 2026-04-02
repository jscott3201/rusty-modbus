//! Integration tests for the TCP transport layer.
//!
//! Tests use localhost loopback — no external network required.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use rusty_modbus_codec::{ResponsePdu, decode_response};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::TcpTransport;
use rusty_modbus_tcp::config::{AccessControl, AccessMode, TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink, TransportStream};
use rusty_modbus_types::MbapHeader;

/// Helper: build a Frame with MBAP header for sending.
fn make_request_frame(txn_id: u16, unit_id: u8, pdu: &[u8]) -> Frame {
    let header = MbapHeader::new(txn_id, unit_id, pdu.len() as u16);
    Frame {
        header: FrameHeader::Mbap(header),
        pdu: Bytes::copy_from_slice(pdu),
    }
}

/// Start a test server on an ephemeral port.
async fn start_server() -> (TcpServerListener, SocketAddr) {
    let config = TcpServerConfig::default();
    let listener = TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}

#[tokio::test]
async fn connect_send_recv_roundtrip() {
    let (listener, addr) = start_server().await;

    // Server task: accept one connection, receive a frame, echo it back.
    let server = tokio::spawn(async move {
        let (mut sink, mut stream, _addr) = listener.accept().await.unwrap();
        let frame = stream.recv().await.unwrap();
        sink.send(frame).await.unwrap();
    });

    // Client: connect, send a frame, receive the echo.
    let config = TcpConfig {
        port: addr.port(),
        ..TcpConfig::default()
    };
    let (mut sink, mut stream) = TcpTransport::connect(config, addr).await.unwrap();

    // Send a ReadHoldingRegisters request: FC 0x03, addr=0x0000, qty=0x0001
    let request_pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
    let frame = make_request_frame(1, 0xFF, &request_pdu);
    sink.send(frame).await.unwrap();

    // Receive the echoed frame.
    let response = stream.recv().await.unwrap();
    assert_eq!(response.unit_id(), 0xFF);
    assert_eq!(&response.pdu[..], &request_pdu);

    server.await.unwrap();
}

#[tokio::test]
async fn full_modbus_roundtrip_through_codec() {
    let (listener, addr) = start_server().await;

    // Server: accept, receive request, send a proper ReadHoldingRegisters response.
    let server = tokio::spawn(async move {
        let (mut sink, mut stream, _addr) = listener.accept().await.unwrap();
        let req_frame = stream.recv().await.unwrap();

        // Build response PDU: FC 0x03, byte_count=2, register=0x1234
        let response_pdu = [0x03, 0x02, 0x12, 0x34];
        let header = MbapHeader::new(
            // Echo the transaction ID from the request.
            match req_frame.header {
                FrameHeader::Mbap(h) => h.transaction_id.get(),
                FrameHeader::Rtu { .. } => 0,
            },
            req_frame.unit_id(),
            response_pdu.len() as u16,
        );
        let resp_frame = Frame {
            header: FrameHeader::Mbap(header),
            pdu: Bytes::copy_from_slice(&response_pdu),
        };
        sink.send(resp_frame).await.unwrap();
    });

    // Client: send request, receive typed response.
    let config = TcpConfig::default();
    let (mut sink, mut stream) = TcpTransport::connect(config, addr).await.unwrap();

    let request_pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
    sink.send(make_request_frame(42, 0xFF, &request_pdu))
        .await
        .unwrap();

    let response = stream.recv().await.unwrap();

    // Decode through the codec layer.
    let decoded = decode_response(&response.pdu).unwrap();
    match decoded {
        ResponsePdu::ReadHoldingRegisters(rhr) => {
            assert_eq!(rhr.count(), 1);
            assert_eq!(rhr.register(0), 0x1234);
        }
        other => panic!("unexpected: {other:?}"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn concurrent_read_write() {
    let (listener, addr) = start_server().await;

    // Server: accept, echo 3 frames.
    let server = tokio::spawn(async move {
        let (mut sink, mut stream, _) = listener.accept().await.unwrap();
        for _ in 0..3 {
            let frame = stream.recv().await.unwrap();
            sink.send(frame).await.unwrap();
        }
    });

    let config = TcpConfig::default();
    let (mut sink, mut stream) = TcpTransport::connect(config, addr).await.unwrap();

    // Writer task sends 3 frames concurrently with reader.
    let writer = tokio::spawn(async move {
        for i in 0..3u16 {
            let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
            sink.send(make_request_frame(i, 0xFF, &pdu)).await.unwrap();
        }
    });

    // Reader collects 3 responses.
    let mut received = Vec::new();
    for _ in 0..3 {
        let frame = stream.recv().await.unwrap();
        received.push(frame);
    }

    writer.await.unwrap();
    server.await.unwrap();

    assert_eq!(received.len(), 3);
}

#[tokio::test]
async fn disconnect_detection() {
    let (listener, addr) = start_server().await;

    // Server: accept then immediately drop.
    let server = tokio::spawn(async move {
        let (_sink, _stream, _) = listener.accept().await.unwrap();
        // Drop — closes connection.
    });

    let config = TcpConfig::default();
    let (_sink, mut stream) = TcpTransport::connect(config, addr).await.unwrap();

    server.await.unwrap();

    // Give the server time to close.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // recv should return Disconnected.
    let result = stream.recv().await;
    assert!(
        matches!(result, Err(rusty_modbus_tcp::TransportError::Disconnected)),
        "expected Disconnected, got {result:?}"
    );
}

#[tokio::test]
async fn connection_counter_tracks_active() {
    let (listener, addr) = start_server().await;
    let counter = listener.connection_counter();

    assert_eq!(counter.load(Ordering::Relaxed), 0);

    let server = tokio::spawn(async move {
        let (_sink, _stream, _) = listener.accept().await.unwrap();
        // Hold connection open.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    let config = TcpConfig::default();
    let (_sink, _stream) = TcpTransport::connect(config, addr).await.unwrap();

    // Counter should be 1 after accept.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(counter.load(Ordering::Relaxed), 1);

    server.await.unwrap();
}

#[tokio::test]
async fn access_control_denies_connection() {
    // Configure to deny all by default.
    let config = TcpServerConfig {
        access_control: Some(AccessControl {
            default_mode: AccessMode::Deny,
            rules: vec![],
        }),
        ..TcpServerConfig::default()
    };

    let listener = TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    // Server: accept in a loop (denied connections are silently dropped internally).
    // We'll timeout after a bit since no valid connection will arrive.
    let server = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept()).await
    });

    // Client: connect (TCP handshake succeeds, but server drops immediately).
    let config = TcpConfig::default();
    let _result = TcpTransport::connect(config, addr).await;

    // Server should timeout since the connection was denied.
    let server_result = server.await.unwrap();
    assert!(server_result.is_err(), "server should have timed out");
}
