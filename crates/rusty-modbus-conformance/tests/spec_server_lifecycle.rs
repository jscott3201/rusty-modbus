//! Project lifecycle policy: server shutdown seals admission before bounded drain.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_server::{
    DataStore, InMemoryStore, ModbusServer, ServerConfig, ServerConfigError, ServerError,
    ShutdownOutcome, StoreConfig,
};
use rusty_modbus_tcp::config::TcpConfig;
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink, TransportStream};
use rusty_modbus_tcp::{TcpTransport, TransportError};
use rusty_modbus_types::{ExceptionCode, MbapHeader, UnitId};
use tokio::sync::Notify;

struct ControlledStore {
    inner: InMemoryStore,
    entered: Notify,
    release: Notify,
    callback_active: AtomicBool,
}

impl ControlledStore {
    fn new() -> Self {
        Self {
            inner: InMemoryStore::new(StoreConfig::default()),
            entered: Notify::new(),
            release: Notify::new(),
            callback_active: AtomicBool::new(false),
        }
    }
}

struct CallbackGuard<'a>(&'a AtomicBool);

impl Drop for CallbackGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl DataStore for ControlledStore {
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
        self.release.notified().await;
        buf[0] = 0x002A;
        Ok(usize::from(quantity.min(1)))
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

fn config(shutdown_timeout: Duration) -> ServerConfig {
    ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UnitId(1),
        shutdown_timeout,
        ..ServerConfig::default()
    }
}

fn read_request() -> Frame {
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(1, 1, 5)),
        pdu: Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x01]),
    }
}

async fn connect(
    address: std::net::SocketAddr,
) -> (rusty_modbus_tcp::TcpSink, rusty_modbus_tcp::TcpRecvStream) {
    TcpTransport::connect(
        TcpConfig {
            read_timeout: None,
            ..TcpConfig::default()
        },
        address,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn invalid_limits_fail_before_bind_without_imposing_client_ring_maximum() {
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    let store = Arc::new(InMemoryStore::new(StoreConfig::default()));
    let invalid = ServerConfig {
        listen_addr: address,
        max_connections: 0,
        ..ServerConfig::default()
    };

    assert!(matches!(
        ModbusServer::start(invalid, Arc::clone(&store)).await,
        Err(ServerError::InvalidConfig(
            ServerConfigError::ZeroMaxConnections
        ))
    ));
    drop(occupied);

    let server = ModbusServer::start(
        ServerConfig {
            max_transactions: 17,
            ..config(Duration::from_secs(1))
        },
        store,
    )
    .await
    .unwrap();
    assert_eq!(server.stop().await, ShutdownOutcome::Drained);
}

#[tokio::test]
async fn admitted_request_finishes_before_clean_shutdown_closes_connection() {
    let store = Arc::new(ControlledStore::new());
    let server = Arc::new(
        ModbusServer::start(config(Duration::from_secs(1)), Arc::clone(&store))
            .await
            .unwrap(),
    );
    let (mut sink, mut stream) = connect(server.local_addr()).await;
    sink.send(read_request()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), store.entered.notified())
        .await
        .unwrap();

    let stop_server = Arc::clone(&server);
    let stop = tokio::spawn(async move { stop_server.stop().await });
    tokio::task::yield_now().await;
    assert!(!stop.is_finished());

    store.release.notify_one();
    assert_eq!(
        stream.recv().await.unwrap().pdu.as_ref(),
        &[0x03, 0x02, 0x00, 0x2A]
    );
    assert_eq!(stop.await.unwrap(), ShutdownOutcome::Drained);
    assert!(matches!(
        stream.recv().await,
        Err(TransportError::Disconnected)
    ));
    assert_eq!(server.metrics().active_connections, 0);
    assert_eq!(server.metrics().active_requests, 0);
}

#[tokio::test]
async fn deadline_forces_blocked_request_and_reclaims_owned_tasks() {
    let store = Arc::new(ControlledStore::new());
    let server = ModbusServer::start(config(Duration::from_millis(25)), Arc::clone(&store))
        .await
        .unwrap();
    let (mut sink, mut stream) = connect(server.local_addr()).await;
    sink.send(read_request()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), store.entered.notified())
        .await
        .unwrap();

    assert_eq!(server.stop().await, ShutdownOutcome::Forced);
    assert!(!store.callback_active.load(Ordering::SeqCst));
    assert_eq!(server.metrics().active_connections, 0);
    assert_eq!(server.metrics().active_requests, 0);
    assert!(matches!(
        stream.recv().await,
        Err(TransportError::Disconnected)
    ));
}
