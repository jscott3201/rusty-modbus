//! TCP Transport conformance tests.
//!
//! Verifies TCP transport behavior against the Modbus Messaging on
//! TCP/IP Implementation Guide V1.0b §4.2–4.4.

use std::net::IpAddr;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::TcpTransport;
use rusty_modbus_tcp::config::{AccessControl, AccessMode, TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink, TransportStream};
use rusty_modbus_types::MbapHeader;

fn make_frame(txn_id: u16, unit_id: u8, pdu: &[u8]) -> Frame {
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(txn_id, unit_id, pdu.len() as u16)),
        pdu: Bytes::copy_from_slice(pdu),
    }
}

// ── §4.2 TCP Config Defaults ──────────────────────────────────────

#[test]
fn spec_4_2_default_port_502() {
    // "The listening TCP port 502 is reserved for MODBUS communications"
    let config = TcpConfig::default();
    assert_eq!(config.port, 502);
}

#[test]
fn spec_4_3_2_tcp_nodelay_enabled() {
    // §4.3.2: "it is recommended to force the TCP-NODELAY option"
    let config = TcpConfig::default();
    assert!(config.tcp_nodelay);
}

#[test]
fn spec_4_3_2_keepalive_enabled() {
    // §4.3.2: "It is recommended to enable the KEEPALIVE option"
    let config = TcpConfig::default();
    assert!(config.keepalive.is_some());
}

#[test]
fn spec_4_3_2_connect_timeout_reasonable() {
    // §4.3.2: "most Berkeley-derived systems set a time limit of 75 seconds...
    //          this default value should be adapted to the real time constraint"
    let config = TcpConfig::default();
    // Our default is 5s — reasonable per spec guidance
    assert!(config.connect_timeout <= Duration::from_secs(10));
    assert!(config.connect_timeout >= Duration::from_secs(1));
}

#[test]
fn spec_server_config_defaults() {
    let config = TcpServerConfig::default();
    assert_eq!(config.max_connections, 64);
    assert!(config.access_control.is_none()); // open by default
}

// ── §4.2.3 Access Control ─────────────────────────────────────────

#[test]
fn spec_4_2_3_access_control_deny_by_default() {
    // "on security mode, the IP addresses not configured by the user are forbidden"
    let ac = AccessControl {
        default_mode: AccessMode::Deny,
        rules: vec![("192.168.1.10".parse::<IpAddr>().unwrap(), AccessMode::Allow)],
    };
    assert!(ac.is_allowed(&"192.168.1.10".parse().unwrap()));
    assert!(!ac.is_allowed(&"192.168.1.11".parse().unwrap()));
    assert!(!ac.is_allowed(&"10.0.0.1".parse().unwrap()));
}

#[test]
fn spec_4_2_3_access_control_allow_by_default() {
    let ac = AccessControl {
        default_mode: AccessMode::Allow,
        rules: vec![("10.0.0.1".parse::<IpAddr>().unwrap(), AccessMode::Deny)],
    };
    assert!(ac.is_allowed(&"192.168.1.10".parse().unwrap()));
    assert!(!ac.is_allowed(&"10.0.0.1".parse().unwrap()));
}

// ── §4.4.1.2 MBAP ADU Example from spec p.22 ─────────────────────

#[tokio::test]
async fn spec_4_4_1_2_mbap_adu_example() {
    // Spec p.22 example: read register #5
    // TxnId=0x0015, ProtoId=0x0000, Length=0x0006, UnitId=0xFF
    // FC=0x03, StartAddr=0x0004, Qty=0x0001
    let (listener, addr) = start_server().await;

    let server = tokio::spawn(async move {
        let (mut sink, mut stream, _, _guard) = listener.accept().await.unwrap();
        let frame = stream.recv().await.unwrap();
        // Verify the received frame matches the spec example
        match frame.header {
            FrameHeader::Mbap(h) => {
                assert_eq!(h.transaction_id.get(), 0x0015);
                assert_eq!(h.protocol_id.get(), 0x0000);
                assert_eq!(h.unit_id, 0xFF);
            }
            _ => panic!("expected MBAP"),
        }
        assert_eq!(&frame.pdu[..], &[0x03, 0x00, 0x04, 0x00, 0x01]);
        // Echo it back
        sink.send(frame).await.unwrap();
    });

    let config = TcpConfig::default();
    let (mut sink, mut stream) = TcpTransport::connect(config, addr).await.unwrap();

    // Send the spec example frame
    let frame = make_frame(0x0015, 0xFF, &[0x03, 0x00, 0x04, 0x00, 0x01]);
    sink.send(frame).await.unwrap();

    let resp = stream.recv().await.unwrap();
    assert_eq!(resp.unit_id(), 0xFF);

    server.await.unwrap();
}

// ── §4.4.1.3 Transaction ID matching ──────────────────────────────

#[tokio::test]
async fn spec_4_4_1_3_transaction_id_preserved() {
    // "the MODBUS server copies in the response the transaction
    //  identifier of the request"
    let (listener, addr) = start_server().await;

    let server = tokio::spawn(async move {
        let (mut sink, mut stream, _, _guard) = listener.accept().await.unwrap();
        let req = stream.recv().await.unwrap();
        // Echo with same transaction ID
        let txn_id = match req.header {
            FrameHeader::Mbap(h) => h.transaction_id.get(),
            _ => panic!("expected MBAP"),
        };
        let resp_header = MbapHeader::new(txn_id, req.unit_id(), 2);
        let resp = Frame {
            header: FrameHeader::Mbap(resp_header),
            pdu: Bytes::from_static(&[0x03, 0x00]),
        };
        sink.send(resp).await.unwrap();
    });

    let config = TcpConfig::default();
    let (mut sink, mut stream) = TcpTransport::connect(config, addr).await.unwrap();

    sink.send(make_frame(0xABCD, 0xFF, &[0x03, 0x00, 0x00, 0x00, 0x01]))
        .await
        .unwrap();

    let resp = stream.recv().await.unwrap();
    match resp.header {
        FrameHeader::Mbap(h) => assert_eq!(h.transaction_id.get(), 0xABCD),
        _ => panic!("expected MBAP"),
    }

    server.await.unwrap();
}

// ── §4.4.1.2 Unit ID semantics ────────────────────────────────────

#[test]
fn spec_4_4_1_2_unit_id_0xff_for_tcp_device() {
    // "On TCP/IP, the MODBUS server is addressed using its IP address;
    //  therefore, the MODBUS Unit Identifier is useless. The value 0xFF has to be used."
    use rusty_modbus_types::UnitId;
    assert!(UnitId(0xFF).is_tcp_device());
}

#[test]
fn spec_4_4_1_2_unit_id_broadcast() {
    // "Address 0 is used as broadcast address."
    // "The value 0 is also accepted to communicate directly to a MODBUS/TCP device."
    use rusty_modbus_types::UnitId;
    assert!(UnitId(0x00).is_broadcast());
}

#[test]
fn spec_4_4_1_2_unit_id_slave_range() {
    // "slave device addresses on serial line are assigned from 1 to 247"
    use rusty_modbus_types::UnitId;
    assert!(UnitId(1).is_valid_slave());
    assert!(UnitId(247).is_valid_slave());
    assert!(!UnitId(0).is_valid_slave());
    assert!(!UnitId(248).is_valid_slave());
}

// ── §4.4.1.2 NumberOfMaxClientTransaction ─────────────────────────

#[test]
fn spec_4_4_1_2_max_client_transactions() {
    // "this parameter can take a value from 1 to 16"
    assert_eq!(rusty_modbus_types::MAX_CLIENT_TRANSACTIONS, 16);
}

// ── Disconnect detection ──────────────────────────────────────────

#[tokio::test]
async fn transport_disconnect_detected() {
    let (listener, addr) = start_server().await;

    let server = tokio::spawn(async move {
        let (_, _, _, _guard) = listener.accept().await.unwrap();
        // Drop — closes connection
    });

    let config = TcpConfig::default();
    let (_sink, mut stream) = TcpTransport::connect(config, addr).await.unwrap();
    server.await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(matches!(
        stream.recv().await,
        Err(rusty_modbus_tcp::TransportError::Disconnected)
    ));
}

async fn start_server() -> (TcpServerListener, std::net::SocketAddr) {
    let config = TcpServerConfig::default();
    let listener = TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    (listener, addr)
}
