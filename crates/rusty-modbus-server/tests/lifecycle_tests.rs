//! Server admission and shutdown lifecycle regressions.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_server::{
    DataStore, InMemoryStore, ModbusServer, ServerConfig, ServerConfigError, ServerError,
    ShutdownOutcome, StoreConfig,
};
use rusty_modbus_tcp::config::TcpConfig;
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink, TransportStream};
use rusty_modbus_tcp::{TcpRecvStream, TcpSink, TcpTransport, TransportError};
use rusty_modbus_types::{ExceptionCode, MbapHeader, UnitId};
use tokio::sync::{Barrier, Notify};

#[derive(Debug, Clone, Copy)]
enum ReadBehavior {
    Block,
    Error,
    Panic,
}

struct GateStore {
    inner: InMemoryStore,
    behavior: ReadBehavior,
    entered: Notify,
    release: Notify,
    callback_active: AtomicBool,
}

impl GateStore {
    fn new(behavior: ReadBehavior) -> Self {
        Self {
            inner: InMemoryStore::new(StoreConfig::default()),
            behavior,
            entered: Notify::new(),
            release: Notify::new(),
            callback_active: AtomicBool::new(false),
        }
    }

    async fn wait_until_entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.entered.notified())
            .await
            .expect("request did not enter the data store");
    }

    fn release(&self) {
        self.release.notify_one();
    }

    fn callback_active(&self) -> bool {
        self.callback_active.load(Ordering::SeqCst)
    }
}

struct CallbackGuard<'a>(&'a AtomicBool);

impl Drop for CallbackGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl DataStore for GateStore {
    fn read_coils(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        self.inner.read_coils(address, quantity, buf)
    }

    fn write_coil(
        &self,
        address: u16,
        value: bool,
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send {
        self.inner.write_coil(address, value)
    }

    fn write_coils(
        &self,
        address: u16,
        values: &[bool],
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send {
        self.inner.write_coils(address, values)
    }

    fn read_discrete_inputs(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [bool],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        self.inner.read_discrete_inputs(address, quantity, buf)
    }

    async fn read_holding_registers(
        &self,
        _address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> Result<usize, ExceptionCode> {
        self.callback_active.store(true, Ordering::SeqCst);
        let _guard = CallbackGuard(&self.callback_active);
        self.entered.notify_one();
        match self.behavior {
            ReadBehavior::Block => {
                self.release.notified().await;
                buf[0] = 0x1234;
                Ok(usize::from(quantity.min(1)))
            }
            ReadBehavior::Error => Err(ExceptionCode::IllegalDataAddress),
            ReadBehavior::Panic => panic!("intentional data store panic"),
        }
    }

    fn write_register(
        &self,
        address: u16,
        value: u16,
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send {
        self.inner.write_register(address, value)
    }

    fn write_registers(
        &self,
        address: u16,
        values: &[u16],
    ) -> impl Future<Output = Result<(), ExceptionCode>> + Send {
        self.inner.write_registers(address, values)
    }

    fn read_input_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        self.inner.read_input_registers(address, quantity, buf)
    }
}

fn server_config(shutdown_timeout: Duration) -> ServerConfig {
    ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        shutdown_timeout,
        ..ServerConfig::default()
    }
}

async fn connect(addr: SocketAddr) -> (TcpSink, TcpRecvStream) {
    TcpTransport::connect(
        TcpConfig {
            read_timeout: None,
            write_timeout: None,
            ..TcpConfig::default()
        },
        addr,
    )
    .await
    .unwrap()
}

fn read_request(transaction_id: u16) -> Frame {
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(transaction_id, 1, 5)),
        pdu: Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x01]),
    }
}

async fn wait_until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition did not become true");
}

async fn assert_exact_rebind(addr: SocketAddr) {
    let rebound = tokio::net::TcpListener::bind(addr)
        .await
        .expect("stopped server must release its listen address");
    drop(rebound);
}

#[tokio::test]
async fn invalid_config_is_rejected_before_bind_and_large_transaction_limit_is_accepted() {
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let occupied_addr = occupied.local_addr().unwrap();
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));

    let mut config = ServerConfig {
        listen_addr: occupied_addr,
        max_connections: 0,
        ..ServerConfig::default()
    };
    assert!(matches!(
        ModbusServer::start(config.clone(), Arc::clone(&store)).await,
        Err(ServerError::InvalidConfig(
            ServerConfigError::ZeroMaxConnections
        ))
    ));

    config.max_connections = 1;
    config.max_transactions = 0;
    assert!(matches!(
        ModbusServer::start(config.clone(), Arc::clone(&store)).await,
        Err(ServerError::InvalidConfig(
            ServerConfigError::ZeroMaxTransactions
        ))
    ));

    config.max_transactions = 1;
    config.shutdown_timeout = Duration::ZERO;
    assert!(matches!(
        ModbusServer::start(config, Arc::clone(&store)).await,
        Err(ServerError::InvalidConfig(
            ServerConfigError::ZeroShutdownTimeout
        ))
    ));
    drop(occupied);

    let server = ModbusServer::start(
        ServerConfig {
            max_transactions: 17,
            ..server_config(Duration::from_secs(1))
        },
        store,
    )
    .await
    .unwrap();
    assert_eq!(server.stop().await, ShutdownOutcome::Drained);
}

#[tokio::test]
async fn maximum_shutdown_timeout_stops_without_panic_and_releases_exact_address() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let server = ModbusServer::start(server_config(Duration::MAX), store)
        .await
        .unwrap();
    let addr = server.local_addr();

    assert_eq!(server.stop().await, ShutdownOutcome::Drained);
    assert_exact_rebind(addr).await;
}

#[tokio::test]
async fn idle_connection_exits_and_clean_stop_releases_exact_address() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let server = ModbusServer::start(server_config(Duration::from_secs(1)), store)
        .await
        .unwrap();
    let addr = server.local_addr();
    let (_sink, _stream) = connect(addr).await;
    wait_until(|| server.metrics().active_connections == 1).await;

    assert_eq!(server.stop().await, ShutdownOutcome::Drained);
    assert_eq!(server.metrics().active_connections, 0);
    assert_eq!(server.metrics().active_requests, 0);
    assert_exact_rebind(addr).await;
}

#[tokio::test]
async fn admitted_request_drains_but_buffered_next_frame_is_rejected() {
    let store = Arc::new(GateStore::new(ReadBehavior::Block));
    let server = Arc::new(
        ModbusServer::start(server_config(Duration::from_secs(1)), Arc::clone(&store))
            .await
            .unwrap(),
    );
    let addr = server.local_addr();
    let (mut sink, mut stream) = connect(addr).await;
    sink.send(read_request(1)).await.unwrap();
    sink.send(read_request(2)).await.unwrap();
    store.wait_until_entered().await;
    assert_eq!(server.metrics().active_requests, 1);

    let stop_server = Arc::clone(&server);
    let stop = tokio::spawn(async move { stop_server.stop().await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!stop.is_finished());

    store.release();
    let response = stream.recv().await.unwrap();
    let FrameHeader::Mbap(header) = response.header else {
        panic!("expected MBAP response");
    };
    assert_eq!(header.transaction_id.get(), 1);
    assert!(matches!(
        stream.recv().await,
        Err(TransportError::Disconnected)
    ));
    assert_eq!(stop.await.unwrap(), ShutdownOutcome::Drained);
    assert_eq!(server.metrics().active_requests, 0);
    assert_eq!(server.metrics().active_connections, 0);
    assert_exact_rebind(addr).await;
}

#[tokio::test]
async fn deadline_aborts_and_joins_blocked_request_before_forced_outcome() {
    let store = Arc::new(GateStore::new(ReadBehavior::Block));
    let server = ModbusServer::start(server_config(Duration::from_millis(50)), Arc::clone(&store))
        .await
        .unwrap();
    let addr = server.local_addr();
    let (mut sink, mut stream) = connect(addr).await;
    sink.send(read_request(1)).await.unwrap();
    store.wait_until_entered().await;

    assert_eq!(server.stop().await, ShutdownOutcome::Forced);
    assert!(!store.callback_active());
    assert_eq!(server.metrics().active_requests, 0);
    assert_eq!(server.metrics().active_connections, 0);
    assert!(matches!(
        stream.recv().await,
        Err(TransportError::Disconnected)
    ));
    assert_exact_rebind(addr).await;
}

#[tokio::test]
async fn cancelled_and_concurrent_stop_callers_share_one_forced_outcome() {
    let store = Arc::new(GateStore::new(ReadBehavior::Block));
    let server = Arc::new(
        ModbusServer::start(
            server_config(Duration::from_millis(100)),
            Arc::clone(&store),
        )
        .await
        .unwrap(),
    );
    let (mut sink, _stream) = connect(server.local_addr()).await;
    sink.send(read_request(1)).await.unwrap();
    store.wait_until_entered().await;

    let first_server = Arc::clone(&server);
    let first = tokio::spawn(async move { first_server.stop().await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());

    let second_server = Arc::clone(&server);
    let third_server = Arc::clone(&server);
    let second = tokio::spawn(async move { second_server.stop().await });
    let third = tokio::spawn(async move { third_server.stop().await });
    assert_eq!(second.await.unwrap(), ShutdownOutcome::Forced);
    assert_eq!(third.await.unwrap(), ShutdownOutcome::Forced);
    assert_eq!(server.stop().await, ShutdownOutcome::Forced);
    assert!(!store.callback_active());
    assert_eq!(server.metrics().active_requests, 0);
    assert_eq!(server.metrics().active_connections, 0);
}

#[tokio::test]
async fn saturation_is_reported_through_server_metrics() {
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let server = ModbusServer::start(
        ServerConfig {
            max_connections: 1,
            ..server_config(Duration::from_secs(1))
        },
        store,
    )
    .await
    .unwrap();
    let addr = server.local_addr();
    let first = tokio::net::TcpStream::connect(addr).await.unwrap();
    wait_until(|| server.metrics().active_connections == 1).await;
    let second = tokio::net::TcpStream::connect(addr).await.unwrap();
    wait_until(|| server.metrics().connection_limit_rejections == 1).await;

    let snapshot = server.metrics();
    assert_eq!(snapshot.accepted_connections, 1);
    assert_eq!(snapshot.active_connections, 1);
    assert_eq!(snapshot.connection_limit_rejections, 1);

    drop((first, second));
    assert_eq!(server.stop().await, ShutdownOutcome::Drained);
    assert_eq!(server.metrics().active_connections, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_racing_with_accepts_leaves_no_admitted_connection_tasks() {
    const ATTEMPTS: usize = 32;

    let server = Arc::new(
        ModbusServer::start(
            ServerConfig {
                max_connections: ATTEMPTS,
                ..server_config(Duration::from_secs(1))
            },
            Arc::new(InMemoryStore::new(StoreConfig::default())),
        )
        .await
        .unwrap(),
    );
    let addr = server.local_addr();
    let barrier = Arc::new(Barrier::new(ATTEMPTS + 2));
    let mut connections = Vec::new();
    for _ in 0..ATTEMPTS {
        let barrier = Arc::clone(&barrier);
        connections.push(tokio::spawn(async move {
            barrier.wait().await;
            tokio::net::TcpStream::connect(addr).await
        }));
    }

    let stop_server = Arc::clone(&server);
    let stop_barrier = Arc::clone(&barrier);
    let stop = tokio::spawn(async move {
        stop_barrier.wait().await;
        stop_server.stop().await
    });
    barrier.wait().await;

    assert_eq!(stop.await.unwrap(), ShutdownOutcome::Drained);
    for connection in connections {
        drop(connection.await.unwrap());
    }
    assert_eq!(server.metrics().active_connections, 0);
    assert_eq!(server.metrics().active_requests, 0);
    assert_exact_rebind(addr).await;
}

#[tokio::test]
async fn request_error_and_panic_release_request_and_connection_counters() {
    let error_store = Arc::new(GateStore::new(ReadBehavior::Error));
    let error_server = ModbusServer::start(
        server_config(Duration::from_secs(1)),
        Arc::clone(&error_store),
    )
    .await
    .unwrap();
    let (mut sink, mut stream) = connect(error_server.local_addr()).await;
    sink.send(read_request(1)).await.unwrap();
    error_store.wait_until_entered().await;
    let response = stream.recv().await.unwrap();
    assert_eq!(response.pdu.as_ref(), &[0x83, 0x02]);
    wait_until(|| error_server.metrics().active_requests == 0).await;
    drop((sink, stream));
    wait_until(|| error_server.metrics().active_connections == 0).await;
    assert_eq!(error_server.stop().await, ShutdownOutcome::Drained);

    let panic_store = Arc::new(GateStore::new(ReadBehavior::Panic));
    let panic_server = ModbusServer::start(
        server_config(Duration::from_secs(1)),
        Arc::clone(&panic_store),
    )
    .await
    .unwrap();
    let (mut sink, mut stream) = connect(panic_server.local_addr()).await;
    sink.send(read_request(2)).await.unwrap();
    panic_store.wait_until_entered().await;
    assert!(matches!(
        stream.recv().await,
        Err(TransportError::Disconnected)
    ));
    wait_until(|| {
        let metrics = panic_server.metrics();
        metrics.active_requests == 0 && metrics.active_connections == 0
    })
    .await;
    assert!(!panic_store.callback_active());
    assert_eq!(panic_server.stop().await, ShutdownOutcome::Drained);
}

#[test]
fn drop_after_runtime_shutdown_is_non_blocking_and_does_not_panic() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(async {
        ModbusServer::start(
            server_config(Duration::from_secs(30)),
            Arc::new(InMemoryStore::new(StoreConfig::default())),
        )
        .await
        .unwrap()
    });
    drop(runtime);

    let started = Instant::now();
    drop(server);
    assert!(started.elapsed() < Duration::from_secs(1));
}
