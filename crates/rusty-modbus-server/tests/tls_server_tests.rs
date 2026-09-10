//! Identity-only TLS serving, not role authorization or pipelining qualification.

#![forbid(unsafe_code)]
#![cfg(feature = "tls")]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use bytes::Bytes;
use rcgen::{CertificateParams, CertifiedIssuer, KeyPair};
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_server::{
    DataStore, IdentityTlsConfigError, InMemoryStore, ShutdownOutcome, StoreConfig,
    TlsModbusServer, TlsModbusServerConfig, TlsServerStartError,
};
use rusty_modbus_tcp::config::{AccessControl, AccessMode};
use rusty_modbus_tcp::{TransportSink, TransportStream};
use rusty_modbus_tls::{TlsClientConfig, TlsRecvStream, TlsServerConfig, TlsSink, TlsTransport};
use rusty_modbus_types::{ExceptionCode, MbapHeader, UnitId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, Semaphore};

struct Certs {
    _dir: tempfile::TempDir,
    server: TlsServerConfig,
    client: TlsClientConfig,
    ca: rustls::pki_types::CertificateDer<'static>,
    client_der: rustls::pki_types::CertificateDer<'static>,
    client_key_der: Vec<u8>,
}

fn certs(expired_client: bool) -> Certs {
    let dir = tempfile::tempdir().unwrap();
    let mut params = CertificateParams::new(vec!["synthetic CA".into()]).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
    let mut params = CertificateParams::new(vec!["localhost".into()]).unwrap();
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap()));
    let server_key = KeyPair::generate().unwrap();
    let server_cert = params.signed_by(&server_key, &*ca).unwrap();
    let mut params = CertificateParams::new(vec!["synthetic client without role".into()]).unwrap();
    if expired_client {
        params.not_before = rcgen::date_time_ymd(2000, 1, 1);
        params.not_after = rcgen::date_time_ymd(2001, 1, 1);
    }
    let client_key = KeyPair::generate().unwrap();
    let client_cert = params.signed_by(&client_key, &*ca).unwrap();
    for (name, pem) in [
        ("ca", ca.pem()),
        ("server", server_cert.pem()),
        ("server-key", server_key.serialize_pem()),
        ("client", client_cert.pem()),
        ("client-key", client_key.serialize_pem()),
    ] {
        std::fs::write(dir.path().join(name), pem).unwrap();
    }
    Certs {
        server: TlsServerConfig {
            server_cert: dir.path().join("server"),
            server_key: dir.path().join("server-key"),
            ca_cert: dir.path().join("ca"),
            ..TlsServerConfig::default()
        },
        client: TlsClientConfig {
            ca_cert: dir.path().join("ca"),
            client_cert: dir.path().join("client"),
            client_key: dir.path().join("client-key"),
            read_timeout: Some(Duration::from_secs(3)),
            write_timeout: Some(Duration::from_secs(3)),
            ..TlsClientConfig::default()
        },
        ca: ca.der().clone(),
        client_der: client_cert.der().clone(),
        client_key_der: client_key.serialize_der(),
        _dir: dir,
    }
}

fn config(certs: &Certs) -> TlsModbusServerConfig {
    let mut config = TlsModbusServerConfig::new(certs.server.clone());
    config.listen_addr = "127.0.0.1:0".parse().unwrap();
    config.read_timeout = None;
    config.shutdown_timeout = Duration::from_secs(1);
    config
}

async fn bounded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(8), future)
        .await
        .expect("bounded test sequence stalled")
}

async fn wait_until(mut predicate: impl FnMut() -> bool) {
    bounded(async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await;
}

async fn connect<S: DataStore + 'static>(
    server: &TlsModbusServer<S>,
    certs: &Certs,
) -> (TlsSink, TlsRecvStream) {
    bounded(TlsTransport::connect(server.local_addr(), &certs.client))
        .await
        .unwrap()
}

fn frame(id: u16, unit: u8, pdu: &[u8]) -> Frame {
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(id, unit, u16::try_from(pdu.len()).unwrap())),
        pdu: Bytes::copy_from_slice(pdu),
    }
}

async fn exchange(
    sink: &mut TlsSink,
    stream: &mut TlsRecvStream,
    id: u16,
    unit: u8,
    pdu: &[u8],
    expected: &[u8],
) {
    bounded(sink.send(frame(id, unit, pdu))).await.unwrap();
    let response = bounded(stream.recv()).await.unwrap();
    let FrameHeader::Mbap(header) = response.header else {
        panic!("expected MBAP");
    };
    assert_eq!(header.transaction_id.get(), id);
    assert_eq!(response.unit_id(), unit);
    assert_eq!(response.pdu.as_ref(), expected);
}

fn assert_idle<S: DataStore + 'static>(server: &TlsModbusServer<S>) {
    let metrics = server.metrics();
    assert_eq!(metrics.server.active_connections, 0);
    assert_eq!(metrics.server.active_requests, 0);
    assert_eq!(metrics.active_handshakes, 0);
    assert_eq!(metrics.active_sessions, 0);
    assert_eq!(
        metrics.handshakes_started,
        metrics.handshakes_succeeded
            + metrics.handshakes_failed
            + metrics.handshakes_timed_out
            + metrics.handshakes_cancelled
    );
}

struct ProbeStore {
    inner: InMemoryStore,
    calls: AtomicUsize,
    active: AtomicUsize,
    entered: Notify,
    block: bool,
    release: Semaphore,
}

impl ProbeStore {
    fn new(block: bool) -> Self {
        Self {
            inner: InMemoryStore::new(StoreConfig::default()),
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            entered: Notify::new(),
            block,
            release: Semaphore::new(0),
        }
    }
}

struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl DataStore for ProbeStore {
    async fn read_coils(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        self.inner.read_coils(address, quantity, out).await
    }
    async fn write_coil(&self, address: u16, value: bool) -> Result<(), ExceptionCode> {
        self.inner.write_coil(address, value).await
    }
    async fn write_coils(&self, address: u16, values: &[bool]) -> Result<(), ExceptionCode> {
        self.inner.write_coils(address, values).await
    }
    async fn read_discrete_inputs(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [bool],
    ) -> Result<usize, ExceptionCode> {
        self.inner
            .read_discrete_inputs(address, quantity, out)
            .await
    }
    async fn read_holding_registers(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.active.fetch_add(1, Ordering::SeqCst);
        let _guard = Active(&self.active);
        self.entered.notify_one();
        if self.block && address == 0 {
            self.release.acquire().await.unwrap().forget();
        }
        self.inner
            .read_holding_registers(address, quantity, out)
            .await
    }
    async fn write_register(&self, address: u16, value: u16) -> Result<(), ExceptionCode> {
        self.inner.write_register(address, value).await
    }
    async fn write_registers(&self, address: u16, values: &[u16]) -> Result<(), ExceptionCode> {
        self.inner.write_registers(address, values).await
    }
    async fn read_input_registers(
        &self,
        address: u16,
        quantity: u16,
        out: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        self.inner
            .read_input_registers(address, quantity, out)
            .await
    }
    async fn handle_custom_function(
        &self,
        _: UnitId,
        _: u8,
        data: &[u8],
        out: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if data == [0xFF] {
            return Ok(out.len() + 1);
        }
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
}

async fn poll_once<F: Future>(future: Pin<&mut F>) -> Option<F::Output> {
    tokio::select! { biased; output = future => Some(output), () = std::future::ready(()) => None }
}

#[tokio::test]
async fn identity_only_profile_rejects_disabled_mutual_tls_before_bind() {
    let mut config = TlsModbusServerConfig::new(Default::default());
    assert_eq!(config.listen_addr.port(), 802);
    config.listen_addr = "127.0.0.1:0".parse().unwrap();
    config.tls.require_client_cert = false;
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    assert!(matches!(
        TlsModbusServer::start(config, store).await,
        Err(TlsServerStartError::InvalidConfig(
            IdentityTlsConfigError::MutualTlsRequired
        ))
    ));
}

#[tokio::test]
async fn invalid_profile_limits_and_certificate_errors_precede_bind() {
    let held = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = held.local_addr().unwrap();
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let called = Arc::new(AtomicBool::new(false));
    let callback_called = Arc::clone(&called);
    let mut config = TlsModbusServerConfig::new(Default::default());
    config.listen_addr = address;
    config.tls.authz_callback = Some(Arc::new(move |_| {
        callback_called.store(true, Ordering::SeqCst);
        rusty_modbus_tls::AuthzDecision::Authorized
    }));
    assert!(matches!(
        TlsModbusServer::start(config, Arc::clone(&store)).await,
        Err(TlsServerStartError::InvalidConfig(
            IdentityTlsConfigError::AuthorizationUnsupported
        ))
    ));
    assert!(!called.load(Ordering::SeqCst));
    for field in [
        "max_connections",
        "tls.max_connections",
        "max_handshakes",
        "handshake_timeout",
        "shutdown_timeout",
        "read_timeout",
        "write_timeout",
    ] {
        let mut config = TlsModbusServerConfig::new(Default::default());
        config.listen_addr = address;
        match field {
            "max_connections" => config.max_connections = 0,
            "tls.max_connections" => config.tls.max_connections = 0,
            "max_handshakes" => config.max_handshakes = 0,
            "handshake_timeout" => config.handshake_timeout = Duration::ZERO,
            "shutdown_timeout" => config.shutdown_timeout = Duration::ZERO,
            "read_timeout" => config.read_timeout = Some(Duration::ZERO),
            _ => config.write_timeout = Some(Duration::ZERO),
        }
        assert!(
            matches!(TlsModbusServer::start(config, Arc::clone(&store)).await,
                         Err(TlsServerStartError::InvalidConfig(IdentityTlsConfigError::ZeroLimit(name))) if name == field)
        );
    }
    for field in ["max_connections", "tls.max_connections", "max_handshakes"] {
        let mut config = TlsModbusServerConfig::new(Default::default());
        config.listen_addr = address;
        match field {
            "max_connections" => config.max_connections = usize::MAX,
            "tls.max_connections" => config.tls.max_connections = usize::MAX,
            _ => config.max_handshakes = usize::MAX,
        }
        assert!(
            matches!(TlsModbusServer::start(config, Arc::clone(&store)).await,
                         Err(TlsServerStartError::InvalidConfig(IdentityTlsConfigError::CapacityTooLarge(name))) if name == field)
        );
    }
    let mut config = TlsModbusServerConfig::new(Default::default());
    config.listen_addr = address;
    assert!(matches!(
        TlsModbusServer::start(config, store).await,
        Err(TlsServerStartError::Tls(_))
    ));
    drop(held);
    TcpListener::bind(address).await.unwrap();
}

#[tokio::test]
async fn trusted_null_role_read_write_compound_and_frame_identity() {
    let certs = certs(false);
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let server = TlsModbusServer::start(config(&certs), store).await.unwrap();
    let (mut sink, mut stream) = connect(&server, &certs).await;
    exchange(
        &mut sink,
        &mut stream,
        0xBEEF,
        1,
        &[6, 0, 0, 0x12, 0x34],
        &[6, 0, 0, 0x12, 0x34],
    )
    .await;
    exchange(
        &mut sink,
        &mut stream,
        2,
        1,
        &[3, 0, 0, 0, 1],
        &[3, 2, 0x12, 0x34],
    )
    .await;
    exchange(
        &mut sink,
        &mut stream,
        3,
        1,
        &[0x17, 0, 0, 0, 1, 0, 0, 0, 1, 2, 0xAB, 0xCD],
        &[0x17, 2, 0xAB, 0xCD],
    )
    .await;
    exchange(
        &mut sink,
        &mut stream,
        4,
        1,
        &[0x16, 0, 0, 0xFF, 0, 0, 0x12],
        &[0x16, 0, 0, 0xFF, 0, 0, 0x12],
    )
    .await;
    exchange(
        &mut sink,
        &mut stream,
        5,
        0xFF,
        &[3, 0, 0, 0, 1],
        &[3, 2, 0xAB, 0x12],
    )
    .await;
    sink.send(frame(6, 0, &[6, 0, 0, 0, 7])).await.unwrap();
    sink.send(frame(7, 2, &[6, 0, 0, 0, 9])).await.unwrap(); // Wrong unit is ignored.
    exchange(
        &mut sink,
        &mut stream,
        8,
        1,
        &[3, 0, 0, 0, 1],
        &[3, 2, 0, 7],
    )
    .await;
    exchange(&mut sink, &mut stream, 9, 1, &[3, 0, 0, 0, 0], &[0x83, 3]).await;
    assert_eq!(bounded(server.stop()).await, ShutdownOutcome::Drained);
    assert_idle(&server);
    assert_eq!(server.metrics().handshakes_succeeded, 1);
}

#[tokio::test]
async fn custom_hook_and_oversize_exception_use_existing_runner() {
    let certs = certs(false);
    let store = Arc::new(ProbeStore::new(false));
    let server = TlsModbusServer::start(config(&certs), Arc::clone(&store))
        .await
        .unwrap();
    let (mut sink, mut stream) = connect(&server, &certs).await;
    exchange(
        &mut sink,
        &mut stream,
        10,
        1,
        &[0x41, 0x12, 0x34],
        &[0x41, 0x12, 0x34],
    )
    .await;
    exchange(&mut sink, &mut stream, 11, 1, &[0x41, 0xFF], &[0xC1, 4]).await;
    sink.send(frame(12, 0, &[0x41, 0x99])).await.unwrap();
    exchange(
        &mut sink,
        &mut stream,
        13,
        1,
        &[3, 0, 0, 0, 1],
        &[3, 2, 0, 0],
    )
    .await;
    assert_eq!(store.calls.load(Ordering::SeqCst), 3); // No custom broadcast callback.
    bounded(server.stop()).await;
    assert_idle(&server);
}

#[tokio::test]
async fn sequential_connection_does_not_block_another_connection() {
    let certs = certs(false);
    let store = Arc::new(ProbeStore::new(true));
    let server = TlsModbusServer::start(config(&certs), Arc::clone(&store))
        .await
        .unwrap();
    let (mut first_sink, mut first_stream) = connect(&server, &certs).await;
    first_sink
        .send(frame(1, 1, &[3, 0, 0, 0, 1]))
        .await
        .unwrap();
    bounded(store.entered.notified()).await;
    first_sink
        .send(frame(2, 1, &[3, 0, 1, 0, 1]))
        .await
        .unwrap();
    let (mut other_sink, mut other_stream) = connect(&server, &certs).await;
    exchange(
        &mut other_sink,
        &mut other_stream,
        3,
        1,
        &[3, 0, 2, 0, 1],
        &[3, 2, 0, 0],
    )
    .await;
    store.release.add_permits(1);
    for id in [1, 2] {
        let response = bounded(first_stream.recv()).await.unwrap();
        let FrameHeader::Mbap(header) = response.header else {
            panic!("MBAP");
        };
        assert_eq!(header.transaction_id.get(), id);
    }
    assert_eq!(store.calls.load(Ordering::SeqCst), 3);
    assert_eq!(bounded(server.stop()).await, ShutdownOutcome::Drained);
    assert_idle(&server);
}

#[tokio::test]
async fn stop_is_caller_cancellation_safe_and_forces_yielding_handlers() {
    for forced in [false, true] {
        let certs = certs(false);
        let store = Arc::new(ProbeStore::new(true));
        let mut config = config(&certs);
        config.shutdown_timeout = if forced {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(2)
        };
        let server = TlsModbusServer::start(config, Arc::clone(&store))
            .await
            .unwrap();
        let (mut sink, mut stream) = connect(&server, &certs).await;
        sink.send(frame(1, 1, &[3, 0, 0, 0, 1])).await.unwrap();
        bounded(store.entered.notified()).await;
        let mut cancelled_stop = Box::pin(server.stop());
        assert!(poll_once(cancelled_stop.as_mut()).await.is_none());
        drop(cancelled_stop);
        // A second queued request must not be admitted after the seal.
        let _ = sink.send(frame(2, 1, &[3, 0, 1, 0, 1])).await;
        if !forced {
            store.release.add_permits(1);
            assert_eq!(
                bounded(stream.recv()).await.unwrap().pdu.as_ref(),
                &[3, 2, 0, 0]
            );
        }
        let expected = if forced {
            ShutdownOutcome::Forced
        } else {
            ShutdownOutcome::Drained
        };
        let (first, second) = bounded(async { tokio::join!(server.stop(), server.stop()) }).await;
        assert_eq!((first, second), (expected, expected));
        assert_eq!(server.stop().await, expected);
        assert_eq!(store.active.load(Ordering::SeqCst), 0);
        assert_eq!(store.calls.load(Ordering::SeqCst), 1);
        assert_idle(&server);
        assert!(TcpStream::connect(server.local_addr()).await.is_err());
        TcpListener::bind(server.local_addr()).await.unwrap();
    }
}

#[tokio::test]
async fn handshake_cap_rejects_excess_without_starving_available_slots() {
    let certs = certs(false);
    let mut config = config(&certs);
    config.max_handshakes = 2;
    config.handshake_timeout = Duration::from_mins(1);
    let server = TlsModbusServer::start(config, Arc::new(ProbeStore::new(false)))
        .await
        .unwrap();
    let stalled = TcpStream::connect(server.local_addr()).await.unwrap();
    wait_until(|| server.metrics().active_handshakes == 1).await;
    let (mut sink, mut stream) = connect(&server, &certs).await;
    exchange(
        &mut sink,
        &mut stream,
        1,
        1,
        &[3, 0, 1, 0, 1],
        &[3, 2, 0, 0],
    )
    .await;
    let second_stalled = TcpStream::connect(server.local_addr()).await.unwrap();
    wait_until(|| server.metrics().active_handshakes == 2).await;
    let mut rejected = TcpStream::connect(server.local_addr()).await.unwrap();
    wait_until(|| server.metrics().handshake_limit_rejections == 1).await;
    let mut byte = [0];
    assert!(matches!(
        bounded(rejected.read(&mut byte)).await,
        Ok(0) | Err(_)
    ));
    assert_eq!(server.metrics().handshakes_started, 3);
    assert_eq!(bounded(server.stop()).await, ShutdownOutcome::Drained);
    assert_idle(&server);
    assert_eq!(server.metrics().handshakes_cancelled, 2);
    drop((stalled, second_stalled, sink, stream));
}

#[tokio::test]
async fn handshake_timeout_and_connection_cap_release_capacity() {
    for tls_is_lower in [false, true] {
        let certs = certs(false);
        let mut config = config(&certs);
        config.max_connections = if tls_is_lower { 3 } else { 1 };
        config.tls.max_connections = if tls_is_lower { 1 } else { 3 };
        config.max_handshakes = 2; // Effective handshake capacity is also one.
        config.handshake_timeout = Duration::from_millis(500);
        let server = TlsModbusServer::start(config, Arc::new(ProbeStore::new(false)))
            .await
            .unwrap();
        let stalled = TcpStream::connect(server.local_addr()).await.unwrap();
        wait_until(|| server.metrics().active_handshakes == 1).await;
        let rejected = TcpStream::connect(server.local_addr()).await.unwrap();
        wait_until(|| server.metrics().server.connection_limit_rejections == 1).await;
        assert_eq!(server.metrics().handshakes_started, 1);
        wait_until(|| {
            server.metrics().handshakes_timed_out == 1
                && server.metrics().server.active_connections == 0
        })
        .await;
        let (mut sink, mut stream) = connect(&server, &certs).await;
        exchange(
            &mut sink,
            &mut stream,
            1,
            1,
            &[3, 0, 1, 0, 1],
            &[3, 2, 0, 0],
        )
        .await;
        let another = TcpStream::connect(server.local_addr()).await.unwrap();
        wait_until(|| server.metrics().server.connection_limit_rejections == 2).await;
        assert_eq!(server.metrics().handshakes_started, 2); // Active TLS session consumes the same cap.
        bounded(server.stop()).await;
        assert_idle(&server);
        drop((stalled, rejected, another));
    }
}

#[tokio::test]
async fn acl_and_plaintext_peers_never_reach_the_store() {
    let certs = certs(false);
    let store = Arc::new(ProbeStore::new(false));
    let mut denied = config(&certs);
    denied.access_control = Some(AccessControl {
        default_mode: AccessMode::Deny,
        rules: vec![],
    });
    let server = TlsModbusServer::start(denied, Arc::clone(&store))
        .await
        .unwrap();
    let socket = TcpStream::connect(server.local_addr()).await.unwrap();
    wait_until(|| server.metrics().server.access_denied_connections == 1).await;
    assert_eq!(server.metrics().handshakes_started, 0);
    bounded(server.stop()).await;
    assert_idle(&server);
    drop(socket);
    let server = TlsModbusServer::start(config(&certs), Arc::clone(&store))
        .await
        .unwrap();
    let mut socket = TcpStream::connect(server.local_addr()).await.unwrap();
    socket
        .write_all(&[0, 1, 0, 0, 0, 6, 1, 3, 0, 0, 0, 1])
        .await
        .unwrap();
    wait_until(|| server.metrics().handshakes_failed == 1).await;
    assert_eq!(store.calls.load(Ordering::SeqCst), 0);
    let (mut sink, mut stream) = connect(&server, &certs).await;
    exchange(
        &mut sink,
        &mut stream,
        2,
        1,
        &[3, 0, 1, 0, 1],
        &[3, 2, 0, 0],
    )
    .await;
    bounded(server.stop()).await;
    assert_idle(&server);
}

#[tokio::test]
async fn untrusted_expired_and_wrong_name_peers_never_reach_the_store() {
    for failure in ["rogue", "expired", "hostname"] {
        let good = certs(false);
        let rogue = certs(false);
        let expired = certs(true);
        let server_certs = if failure == "expired" {
            &expired
        } else {
            &good
        };
        let store = Arc::new(ProbeStore::new(false));
        let server = TlsModbusServer::start(config(server_certs), Arc::clone(&store))
            .await
            .unwrap();
        let mut client = server_certs.client.clone();
        if failure == "rogue" {
            client.client_cert = rogue.client.client_cert.clone();
            client.client_key = rogue.client.client_key.clone();
        }
        if failure == "hostname" {
            client.server_name = Some("wrong.invalid".into());
        }
        if let Ok((mut sink, mut stream)) =
            bounded(TlsTransport::connect(server.local_addr(), &client)).await
        {
            // TLS 1.3 may surface client-auth rejection only on subsequent I/O.
            let result = sink.send(frame(1, 1, &[3, 0, 0, 0, 1])).await;
            if result.is_ok() {
                assert!(bounded(stream.recv()).await.is_err());
            }
        }
        wait_until(|| server.metrics().handshakes_failed == 1).await;
        assert_eq!(store.calls.load(Ordering::SeqCst), 0);
        bounded(server.stop()).await;
        assert_idle(&server);
    }
}

#[tokio::test]
async fn missing_client_certificate_is_rejected_by_the_serving_path() {
    let certs = certs(false);
    let store = Arc::new(ProbeStore::new(false));
    let server = TlsModbusServer::start(config(&certs), Arc::clone(&store))
        .await
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certs.ca.clone()).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let socket = TcpStream::connect(server.local_addr()).await.unwrap();
    if let Ok(mut tls) = bounded(connector.connect(
        rustls::pki_types::ServerName::try_from("localhost").unwrap(),
        socket,
    ))
    .await
    {
        let sent = tls.write_all(&[0, 1, 0, 0, 0, 6, 1, 3, 0, 0, 0, 1]).await;
        if sent.is_ok() {
            assert!(matches!(
                bounded(tls.read(&mut [0; 12])).await,
                Ok(0) | Err(_)
            ));
        }
    }
    wait_until(|| server.metrics().handshakes_failed == 1).await;
    assert_eq!(store.calls.load(Ordering::SeqCst), 0);
    bounded(server.stop()).await;
    assert_idle(&server);
}

#[tokio::test]
async fn serving_negotiates_tls13_and_rejects_tls12() {
    let certs = certs(false);
    let server = TlsModbusServer::start(config(&certs), Arc::new(ProbeStore::new(false)))
        .await
        .unwrap();
    for tls13 in [true, false] {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certs.ca.clone()).unwrap();
        let versions = if tls13 {
            vec![&rustls::version::TLS13]
        } else {
            vec![&rustls::version::TLS12]
        };
        let client = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&versions)
        .unwrap()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![certs.client_der.clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certs.client_key_der.clone()).into(),
        )
        .unwrap();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
        let socket = TcpStream::connect(server.local_addr()).await.unwrap();
        let result = bounded(connector.connect(
            rustls::pki_types::ServerName::try_from("localhost").unwrap(),
            socket,
        ))
        .await;
        if tls13 {
            let stream = result.unwrap();
            assert_eq!(
                stream.get_ref().1.protocol_version(),
                Some(rustls::ProtocolVersion::TLSv1_3)
            );
        } else {
            assert!(result.is_err());
        }
    }
    wait_until(|| server.metrics().handshakes_failed == 1).await;
    bounded(server.stop()).await;
    assert_idle(&server);
}

#[tokio::test]
async fn single_handshake_cap_rejects_without_waiting_tasks() {
    let certs = certs(false);
    let mut settings = config(&certs);
    settings.max_connections = 4;
    settings.max_handshakes = 1;
    settings.handshake_timeout = Duration::from_mins(1);
    let server = TlsModbusServer::start(settings, Arc::new(ProbeStore::new(false)))
        .await
        .unwrap();
    let first = TcpStream::connect(server.local_addr()).await.unwrap();
    wait_until(|| server.metrics().active_handshakes == 1).await;
    let second = TcpStream::connect(server.local_addr()).await.unwrap();
    wait_until(|| server.metrics().handshake_limit_rejections == 1).await;
    assert_eq!(server.metrics().handshakes_started, 1);
    assert_eq!(server.metrics().server.connection_limit_rejections, 0);
    assert_eq!(bounded(server.stop()).await, ShutdownOutcome::Drained);
    assert_idle(&server);
    drop((first, second));
}

#[tokio::test]
async fn receive_timeout_disconnect_and_drop_release_tasks() {
    let certs = certs(false);
    let mut settings = config(&certs);
    settings.read_timeout = Some(Duration::from_millis(100));
    let server = TlsModbusServer::start(settings, Arc::new(ProbeStore::new(false)))
        .await
        .unwrap();
    let (sink, mut stream) = connect(&server, &certs).await;
    assert!(bounded(stream.recv()).await.is_err());
    wait_until(|| server.metrics().server.active_connections == 0).await;
    bounded(server.stop()).await;
    assert_idle(&server);
    drop(sink);

    let store = Arc::new(ProbeStore::new(true));
    let server = TlsModbusServer::start(config(&certs), Arc::clone(&store))
        .await
        .unwrap();
    let address = server.local_addr();
    let (mut sink, stream) = connect(&server, &certs).await;
    sink.send(frame(1, 1, &[3, 0, 0, 0, 1])).await.unwrap();
    bounded(store.entered.notified()).await;
    drop((sink, stream));
    store.release.add_permits(1);
    wait_until(|| server.metrics().server.active_connections == 0).await;
    let stalled = TcpStream::connect(address).await.unwrap();
    wait_until(|| server.metrics().active_handshakes == 1).await;
    drop(server); // Nonwaiting cancellation, not an immediate-rebind assertion.
    bounded(async {
        loop {
            if let Ok(listener) = TcpListener::bind(address).await {
                drop(listener);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(store.active.load(Ordering::SeqCst), 0);
    drop(stalled);
}
