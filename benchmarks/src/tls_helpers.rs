//! TLS certificate generation and transport setup for benchmarks.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_server::handler;
use rusty_modbus_server::store::DataStore;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_tls::config::{TlsClientConfig, TlsServerConfig};
use rusty_modbus_tls::{TlsServerListener, TlsTransport};
use rusty_modbus_types::{MbapHeader, UnitId};
use rcgen::{CertificateParams, CertifiedIssuer, KeyPair};
use tempfile::NamedTempFile;
use tokio::task::JoinHandle;

/// Holds temp file handles for test certificates (files stay alive as long as struct is alive).
pub struct TestCerts {
    pub ca_cert: NamedTempFile,
    pub server_cert: NamedTempFile,
    pub server_key: NamedTempFile,
    pub client_cert: NamedTempFile,
    pub client_key: NamedTempFile,
}

/// Generate CA + server + client certs via rcgen, write PEM to temp files.
pub fn generate_test_certs() -> TestCerts {
    let mut ca_params = CertificateParams::new(vec!["Test CA".to_string()]).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

    let mut server_params = CertificateParams::new(vec!["127.0.0.1".to_string()]).unwrap();
    server_params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::LOCALHOST,
        )));
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server_params
        .signed_by(&server_key, &*ca)
        .unwrap();

    let client_params = CertificateParams::new(vec!["Test Client".to_string()]).unwrap();
    let client_key = KeyPair::generate().unwrap();
    let client_cert = client_params
        .signed_by(&client_key, &*ca)
        .unwrap();

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
        ca_cert: ca_file,
        server_cert: server_cert_file,
        server_key: server_key_file,
        client_cert: client_cert_file,
        client_key: client_key_file,
    }
}

/// Start a TLS server with handler dispatch. Returns join handle + bound address.
pub async fn make_tls_server<S: DataStore + 'static>(
    certs: &TestCerts,
    store: Arc<S>,
) -> (JoinHandle<()>, SocketAddr) {
    let config = TlsServerConfig {
        server_cert: certs.server_cert.path().to_path_buf(),
        server_key: certs.server_key.path().to_path_buf(),
        ca_cert: certs.ca_cert.path().to_path_buf(),
        require_client_cert: true,
        ..TlsServerConfig::default()
    };

    let listener = TlsServerListener::bind("127.0.0.1:0".parse().unwrap(), &config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _)) = listener.accept().await {
            let conn_store = Arc::clone(&store);
            tokio::spawn(async move {
                while let Ok(frame) = stream.recv().await {
                    let txn_id = match frame.header {
                        FrameHeader::Mbap(h) => h.transaction_id.get(),
                        FrameHeader::Rtu { .. } => 0,
                    };
                    let unit_id = UnitId(frame.unit_id());
                    if let Some(resp_pdu) =
                        handler::process_request(&frame.pdu, unit_id, conn_store.as_ref(), &rusty_modbus_server::DeviceIdentification::default())
                            .await
                    {
                        let header =
                            MbapHeader::new(txn_id, unit_id.0, resp_pdu.len() as u16);
                        let resp = Frame {
                            header: FrameHeader::Mbap(header),
                            pdu: Bytes::from(resp_pdu),
                        };
                        if sink.send(resp).await.is_err() {
                            break;
                        }
                    }
                }
            });
        }
    });

    (handle, addr)
}

/// Connect a TLS client, returning raw split halves.
pub async fn make_tls_client(
    certs: &TestCerts,
    addr: SocketAddr,
) -> (rusty_modbus_tls::TlsSink, rusty_modbus_tls::TlsRecvStream) {
    let config = TlsClientConfig {
        ca_cert: certs.ca_cert.path().to_path_buf(),
        client_cert: certs.client_cert.path().to_path_buf(),
        client_key: certs.client_key.path().to_path_buf(),
        connect_timeout: Duration::from_secs(5),
        read_timeout: Some(Duration::from_secs(5)),
        write_timeout: Some(Duration::from_secs(5)),
    };
    TlsTransport::connect(addr, &config).await.unwrap()
}
