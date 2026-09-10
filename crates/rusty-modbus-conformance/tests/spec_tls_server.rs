//! Identity-only TLS serving foundation: authenticated transport and bounded lifecycle.
//! Not role authorization, strict role validation, pipelining, or full Security-profile proof.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use rcgen::{CertificateParams, CertifiedIssuer, KeyPair};
use rusty_modbus_client::{ClientConfig, ModbusClient};
use rusty_modbus_server::{
    IdentityTlsConfigError, InMemoryStore, ShutdownOutcome, StoreConfig, TlsModbusServer,
    TlsModbusServerConfig, TlsServerStartError,
};
use rusty_modbus_tls::{TlsClientConfig, TlsServerConfig, TlsTransport};
use rusty_modbus_types::UnitId;

fn fixture() -> (tempfile::TempDir, TlsModbusServerConfig, TlsClientConfig) {
    let dir = tempfile::tempdir().unwrap();
    let mut params = CertificateParams::new(vec!["conformance synthetic CA".into()]).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
    let mut server = CertificateParams::new(vec!["localhost".into()]).unwrap();
    server
        .subject_alt_names
        .push(rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap()));
    let server_key = KeyPair::generate().unwrap();
    let server_cert = server.signed_by(&server_key, &*ca).unwrap();
    let client_key = KeyPair::generate().unwrap();
    let client_cert = CertificateParams::new(vec!["null role client".into()])
        .unwrap()
        .signed_by(&client_key, &*ca)
        .unwrap();
    for (name, pem) in [
        ("ca", ca.pem()),
        ("server", server_cert.pem()),
        ("server-key", server_key.serialize_pem()),
        ("client", client_cert.pem()),
        ("client-key", client_key.serialize_pem()),
    ] {
        std::fs::write(dir.path().join(name), pem).unwrap();
    }
    let tls = TlsServerConfig {
        server_cert: dir.path().join("server"),
        server_key: dir.path().join("server-key"),
        ca_cert: dir.path().join("ca"),
        ..TlsServerConfig::default()
    };
    let mut server = TlsModbusServerConfig::new(tls);
    server.listen_addr = "127.0.0.1:0".parse().unwrap();
    server.max_connections = 2;
    server.max_handshakes = 2;
    server.read_timeout = None;
    server.handshake_timeout = Duration::from_mins(1);
    let client = TlsClientConfig {
        ca_cert: dir.path().join("ca"),
        client_cert: dir.path().join("client"),
        client_key: dir.path().join("client-key"),
        ..TlsClientConfig::default()
    };
    (dir, server, client)
}

#[tokio::test]
async fn identity_only_mtls_serves_normal_modbus_without_claiming_role_authorization() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (_files, config, client_config) = fixture();
        let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
        let mut invalid = config.clone();
        invalid.tls.require_client_cert = false;
        assert!(matches!(
            TlsModbusServer::start(invalid, Arc::clone(&store)).await,
            Err(TlsServerStartError::InvalidConfig(
                IdentityTlsConfigError::MutualTlsRequired
            ))
        ));
        let mut invalid = config.clone();
        invalid.tls.authz_callback = Some(Arc::new(|_| {
            panic!("unsupported authorization must not execute")
        }));
        assert!(matches!(
            TlsModbusServer::start(invalid, Arc::clone(&store)).await,
            Err(TlsServerStartError::InvalidConfig(
                IdentityTlsConfigError::AuthorizationUnsupported
            ))
        ));
        let server = TlsModbusServer::start(config, store).await.unwrap();
        let (sink, stream) = TlsTransport::connect(server.local_addr(), &client_config)
            .await
            .unwrap();
        let client = ModbusClient::from_transport(sink, stream, ClientConfig::default());
        client
            .write_single_register(UnitId(1), 7, 0xBEEF)
            .await
            .unwrap();
        assert_eq!(
            client
                .read_holding_registers(UnitId(1), 7, 1)
                .await
                .unwrap(),
            vec![0xBEEF]
        );
        client.shutdown().await;
        assert_eq!(server.stop().await, ShutdownOutcome::Drained);
        let metrics = server.metrics();
        assert_eq!(metrics.handshakes_succeeded, 1);
        assert_eq!(metrics.server.active_connections, 0);
        assert_eq!(metrics.server.active_requests, 0);
        assert_eq!(metrics.active_handshakes, 0);
        assert_eq!(metrics.active_sessions, 0);
    })
    .await
    .expect("TLS serving conformance sequence stalled");
}

#[tokio::test]
async fn stalled_tls_handshake_consumes_admission_and_stop_reclaims_it() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (_files, mut config, _) = fixture();
        config.max_connections = 1;
        let server =
            TlsModbusServer::start(config, Arc::new(InMemoryStore::new(StoreConfig::default())))
                .await
                .unwrap();
        let stalled = tokio::net::TcpStream::connect(server.local_addr())
            .await
            .unwrap();
        while server.metrics().active_handshakes != 1 {
            tokio::task::yield_now().await;
        }
        let rejected = tokio::net::TcpStream::connect(server.local_addr())
            .await
            .unwrap();
        while server.metrics().server.connection_limit_rejections == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(server.metrics().handshakes_started, 1);
        assert_eq!(server.stop().await, ShutdownOutcome::Drained);
        assert_eq!(server.metrics().handshakes_cancelled, 1);
        assert_eq!(server.metrics().active_handshakes, 0);
        assert_eq!(server.metrics().server.active_connections, 0);
        assert!(
            tokio::net::TcpStream::connect(server.local_addr())
                .await
                .is_err()
        );
        drop((stalled, rejected));
    })
    .await
    .expect("stalled TLS shutdown sequence exceeded watchdog");
}
