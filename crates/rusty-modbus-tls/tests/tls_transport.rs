//! Integration tests for TLS transport with mutual authentication.

use std::io::Write;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_tls::config::{TlsClientConfig, TlsServerConfig};
use rusty_modbus_tls::{TlsServerListener, TlsTransport};
use rusty_modbus_types::MbapHeader;
use rcgen::{CertificateParams, CertifiedIssuer, KeyPair};
use tempfile::NamedTempFile;

struct TestCerts {
    ca_cert_path: NamedTempFile,
    server_cert_path: NamedTempFile,
    server_key_path: NamedTempFile,
    client_cert_path: NamedTempFile,
    client_key_path: NamedTempFile,
}

/// Generate self-signed CA, server, and client certificates for testing.
fn generate_test_certs() -> TestCerts {
    // Generate CA.
    let mut ca_params = CertificateParams::new(vec!["Test CA".to_string()]).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

    // Generate server cert signed by CA.
    let mut server_params = CertificateParams::new(vec!["127.0.0.1".to_string()]).unwrap();
    server_params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::LOCALHOST,
        )));
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server_params.signed_by(&server_key, &*ca).unwrap();

    // Generate client cert signed by CA.
    let client_params = CertificateParams::new(vec!["Test Client".to_string()]).unwrap();
    let client_key = KeyPair::generate().unwrap();
    let client_cert = client_params.signed_by(&client_key, &*ca).unwrap();

    // Write to temp files.
    let mut ca_file = NamedTempFile::new().unwrap();
    ca_file.write_all(ca.pem().as_bytes()).unwrap();

    let mut server_cert_file = NamedTempFile::new().unwrap();
    server_cert_file
        .write_all(server_cert.pem().as_bytes())
        .unwrap();

    let mut server_key_file = NamedTempFile::new().unwrap();
    server_key_file
        .write_all(server_key.serialize_pem().as_bytes())
        .unwrap();

    let mut client_cert_file = NamedTempFile::new().unwrap();
    client_cert_file
        .write_all(client_cert.pem().as_bytes())
        .unwrap();

    let mut client_key_file = NamedTempFile::new().unwrap();
    client_key_file
        .write_all(client_key.serialize_pem().as_bytes())
        .unwrap();

    TestCerts {
        ca_cert_path: ca_file,
        server_cert_path: server_cert_file,
        server_key_path: server_key_file,
        client_cert_path: client_cert_file,
        client_key_path: client_key_file,
    }
}

fn make_frame(txn: u16, unit: u8, pdu: &[u8]) -> Frame {
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(txn, unit, pdu.len() as u16)),
        pdu: Bytes::copy_from_slice(pdu),
    }
}

#[tokio::test]
async fn mutual_auth_roundtrip() {
    let certs = generate_test_certs();

    let server_config = TlsServerConfig {
        server_cert: certs.server_cert_path.path().to_path_buf(),
        server_key: certs.server_key_path.path().to_path_buf(),
        ca_cert: certs.ca_cert_path.path().to_path_buf(),
        require_client_cert: true,
        ..TlsServerConfig::default()
    };

    let listener = TlsServerListener::bind("127.0.0.1:0".parse().unwrap(), &server_config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    // Server: accept, echo one frame.
    let server = tokio::spawn(async move {
        let (mut sink, mut stream, _) = listener.accept().await.unwrap();
        let frame = stream.recv().await.unwrap();
        sink.send(frame).await.unwrap();
    });

    let client_config = TlsClientConfig {
        ca_cert: certs.ca_cert_path.path().to_path_buf(),
        client_cert: certs.client_cert_path.path().to_path_buf(),
        client_key: certs.client_key_path.path().to_path_buf(),
        connect_timeout: Duration::from_secs(5),
        read_timeout: Some(Duration::from_secs(5)),
        write_timeout: Some(Duration::from_secs(5)),
    };

    let (mut sink, mut stream) = TlsTransport::connect(addr, &client_config).await.unwrap();

    let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
    sink.send(make_frame(1, 0xFF, &pdu)).await.unwrap();

    let resp = stream.recv().await.unwrap();
    assert_eq!(resp.unit_id(), 0xFF);
    assert_eq!(&resp.pdu[..], &pdu);

    server.await.unwrap();
}

#[tokio::test]
async fn full_modbus_over_tls() {
    let certs = generate_test_certs();

    let server_config = TlsServerConfig {
        server_cert: certs.server_cert_path.path().to_path_buf(),
        server_key: certs.server_key_path.path().to_path_buf(),
        ca_cert: certs.ca_cert_path.path().to_path_buf(),
        require_client_cert: true,
        ..TlsServerConfig::default()
    };

    let listener = TlsServerListener::bind("127.0.0.1:0".parse().unwrap(), &server_config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    // Server: accept, respond with ReadHoldingRegisters response.
    let server = tokio::spawn(async move {
        let (mut sink, mut stream, _) = listener.accept().await.unwrap();
        let req = stream.recv().await.unwrap();
        let txn_id = match req.header {
            FrameHeader::Mbap(h) => h.transaction_id.get(),
            _ => 0,
        };
        let resp_pdu = [0x03, 0x04, 0xDE, 0xAD, 0xBE, 0xEF];
        let header = MbapHeader::new(txn_id, req.unit_id(), resp_pdu.len() as u16);
        let resp = Frame {
            header: FrameHeader::Mbap(header),
            pdu: Bytes::from(resp_pdu.to_vec()),
        };
        sink.send(resp).await.unwrap();
    });

    let client_config = TlsClientConfig {
        ca_cert: certs.ca_cert_path.path().to_path_buf(),
        client_cert: certs.client_cert_path.path().to_path_buf(),
        client_key: certs.client_key_path.path().to_path_buf(),
        connect_timeout: Duration::from_secs(5),
        read_timeout: Some(Duration::from_secs(5)),
        write_timeout: Some(Duration::from_secs(5)),
    };

    let (mut sink, mut stream) = TlsTransport::connect(addr, &client_config).await.unwrap();

    let pdu = [0x03, 0x00, 0x00, 0x00, 0x02];
    sink.send(make_frame(42, 0x01, &pdu)).await.unwrap();

    let resp = stream.recv().await.unwrap();

    // Decode through codec.
    let decoded = rusty_modbus_codec::decode_response(&resp.pdu).unwrap();
    match decoded {
        rusty_modbus_codec::ResponsePdu::ReadHoldingRegisters(rhr) => {
            assert_eq!(rhr.count(), 2);
            assert_eq!(rhr.register(0), 0xDEAD);
            assert_eq!(rhr.register(1), 0xBEEF);
        }
        other => panic!("unexpected: {other:?}"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn missing_client_cert_fails() {
    let certs = generate_test_certs();

    let server_config = TlsServerConfig {
        server_cert: certs.server_cert_path.path().to_path_buf(),
        server_key: certs.server_key_path.path().to_path_buf(),
        ca_cert: certs.ca_cert_path.path().to_path_buf(),
        require_client_cert: true,
        ..TlsServerConfig::default()
    };

    let listener = TlsServerListener::bind("127.0.0.1:0".parse().unwrap(), &server_config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    // Server: try to accept (will fail due to missing client cert).
    let server = tokio::spawn(async move {
        let result = listener.accept().await;
        result.is_err() // Should fail
    });

    // Client: connect WITHOUT a valid client cert — use the server cert as "client cert"
    // which won't be signed by the CA the server expects.
    // Actually, rustls requires client cert to be present. The best test is
    // to provide a cert not signed by the expected CA.
    // For simplicity, create a separate self-signed cert not in the CA chain.
    let rogue_params = CertificateParams::new(vec!["Rogue".to_string()]).unwrap();
    let rogue_key = KeyPair::generate().unwrap();
    let rogue_cert = rogue_params.self_signed(&rogue_key).unwrap();

    let mut rogue_cert_file = NamedTempFile::new().unwrap();
    rogue_cert_file.write_all(rogue_cert.pem().as_bytes()).unwrap();
    let mut rogue_key_file = NamedTempFile::new().unwrap();
    rogue_key_file.write_all(rogue_key.serialize_pem().as_bytes()).unwrap();

    let client_config = TlsClientConfig {
        ca_cert: certs.ca_cert_path.path().to_path_buf(),
        client_cert: rogue_cert_file.path().to_path_buf(),
        client_key: rogue_key_file.path().to_path_buf(),
        connect_timeout: Duration::from_secs(2),
        ..TlsClientConfig::default()
    };

    // The handshake should fail because the server won't accept the rogue cert.
    let result = TlsTransport::connect(addr, &client_config).await;
    // Either connect fails or the server rejects — both are acceptable.
    // Give server time to process.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let _server_failed = server.await.unwrap();
    // At least one side should have failed.
    assert!(result.is_err() || _server_failed);
}
