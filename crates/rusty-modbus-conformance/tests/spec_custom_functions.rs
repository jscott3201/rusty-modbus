//! Server extension behavior for non-standard function codes.

use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::future::join_all;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_server::{
    DataStore, DeviceIdentification, InMemoryStore, ModbusServer, ServerConfig, ShutdownOutcome,
    StoreConfig, handler,
};
use rusty_modbus_tcp::config::TcpConfig;
use rusty_modbus_tcp::transport::{TransportConnect, TransportSink, TransportStream};
use rusty_modbus_tcp::{TcpTransport, TransportError};
use rusty_modbus_types::{ExceptionCode, MAX_PDU_SIZE, MbapHeader, UnitId};
use tokio::sync::Barrier;

const CUSTOM_FC: u8 = 0x41;
const UNIT: UnitId = UnitId(7);

enum CustomBehavior {
    Fixed(Vec<u8>),
    EchoRequest,
    Overreport,
    Error(ExceptionCode),
    Panic,
    ConcurrentEcho(Arc<Barrier>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Observation {
    unit_id: UnitId,
    function_code: u8,
    request_data: Vec<u8>,
    response_capacity: usize,
    response_address: usize,
}

struct CustomStore {
    inner: InMemoryStore,
    behavior: CustomBehavior,
    calls: AtomicUsize,
    observations: Mutex<Vec<Observation>>,
}

impl CustomStore {
    fn new(behavior: CustomBehavior) -> Self {
        Self {
            inner: InMemoryStore::new(StoreConfig::default()),
            behavior,
            calls: AtomicUsize::new(0),
            observations: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn observations(&self) -> Vec<Observation> {
        self.observations.lock().unwrap().clone()
    }
}

impl DataStore for CustomStore {
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

    fn read_holding_registers(
        &self,
        address: u16,
        quantity: u16,
        buf: &mut [u16],
    ) -> impl Future<Output = Result<usize, ExceptionCode>> + Send {
        self.inner.read_holding_registers(address, quantity, buf)
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

    async fn handle_custom_function(
        &self,
        unit_id: UnitId,
        function_code: u8,
        request_data: &[u8],
        response_data: &mut [u8],
    ) -> Result<usize, ExceptionCode> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.observations.lock().unwrap().push(Observation {
            unit_id,
            function_code,
            request_data: request_data.to_vec(),
            response_capacity: response_data.len(),
            response_address: response_data.as_ptr() as usize,
        });

        match &self.behavior {
            CustomBehavior::Fixed(data) => {
                response_data[..data.len()].copy_from_slice(data);
                Ok(data.len())
            }
            CustomBehavior::EchoRequest => {
                response_data[..request_data.len()].copy_from_slice(request_data);
                Ok(request_data.len())
            }
            CustomBehavior::Overreport => Ok(response_data.len() + 1),
            CustomBehavior::Error(code) => Err(*code),
            CustomBehavior::Panic => panic!("intentional custom function panic"),
            CustomBehavior::ConcurrentEcho(barrier) => {
                barrier.wait().await;
                response_data[..request_data.len()].copy_from_slice(request_data);
                Ok(request_data.len())
            }
        }
    }
}

async fn respond<S: DataStore>(store: &S, pdu: &[u8], unit_id: UnitId) -> Option<Vec<u8>> {
    handler::process_request(pdu, unit_id, store, &DeviceIdentification::default()).await
}

fn server_config() -> ServerConfig {
    ServerConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        unit_id: UNIT,
        shutdown_timeout: Duration::from_secs(1),
        ..ServerConfig::default()
    }
}

async fn connect(
    address: std::net::SocketAddr,
) -> (rusty_modbus_tcp::TcpSink, rusty_modbus_tcp::TcpRecvStream) {
    TcpTransport::connect(
        TcpConfig {
            read_timeout: Some(Duration::from_secs(1)),
            write_timeout: Some(Duration::from_secs(1)),
            ..TcpConfig::default()
        },
        address,
    )
    .await
    .unwrap()
}

async fn wait_until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition did not become true");
}

#[tokio::test]
async fn default_store_returns_illegal_function() {
    let store = InMemoryStore::new(StoreConfig::default());

    assert_eq!(
        respond(&store, &[CUSTOM_FC, 0xAA], UNIT).await,
        Some(vec![
            CUSTOM_FC | 0x80,
            ExceptionCode::IllegalFunction.code()
        ])
    );
}

#[tokio::test]
async fn hook_receives_context_and_server_prepends_function_code() {
    let store = CustomStore::new(CustomBehavior::Fixed(vec![0x10, 0x20, 0x30]));

    assert_eq!(
        respond(&store, &[CUSTOM_FC, 0xAA, 0xBB], UNIT).await,
        Some(vec![CUSTOM_FC, 0x10, 0x20, 0x30])
    );
    let observations = store.observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].unit_id, UNIT);
    assert_eq!(observations[0].function_code, CUSTOM_FC);
    assert_eq!(observations[0].request_data, [0xAA, 0xBB]);
    assert_eq!(observations[0].response_capacity, MAX_PDU_SIZE - 1);
}

#[tokio::test]
async fn zero_length_response_contains_only_function_code() {
    let store = CustomStore::new(CustomBehavior::Fixed(Vec::new()));

    assert_eq!(
        respond(&store, &[CUSTOM_FC], UNIT).await,
        Some(vec![CUSTOM_FC])
    );
}

#[tokio::test]
async fn maximum_request_data_reaches_hook() {
    let store = CustomStore::new(CustomBehavior::EchoRequest);
    let mut request = vec![CUSTOM_FC];
    request.extend(std::iter::repeat_n(0xA5, MAX_PDU_SIZE - 1));

    assert_eq!(respond(&store, &request, UNIT).await, Some(request.clone()));
    assert_eq!(store.observations()[0].request_data, request[1..]);
}

#[tokio::test]
async fn maximum_response_data_produces_maximum_pdu() {
    let data = vec![0x5A; MAX_PDU_SIZE - 1];
    let store = CustomStore::new(CustomBehavior::Fixed(data.clone()));
    let response = respond(&store, &[CUSTOM_FC], UNIT).await.unwrap();

    assert_eq!(response.len(), MAX_PDU_SIZE);
    assert_eq!(response[0], CUSTOM_FC);
    assert_eq!(response[1..], data);
}

#[tokio::test]
async fn overreported_response_length_returns_server_device_failure() {
    let store = CustomStore::new(CustomBehavior::Overreport);

    assert_eq!(
        respond(&store, &[CUSTOM_FC], UNIT).await,
        Some(vec![
            CUSTOM_FC | 0x80,
            ExceptionCode::ServerDeviceFailure.code(),
        ])
    );
}

#[tokio::test]
async fn hook_exception_is_encoded_exactly() {
    let store = CustomStore::new(CustomBehavior::Error(ExceptionCode::IllegalDataAddress));

    assert_eq!(
        respond(&store, &[CUSTOM_FC, 0x01], UNIT).await,
        Some(vec![
            CUSTOM_FC | 0x80,
            ExceptionCode::IllegalDataAddress.code(),
        ])
    );
}

#[tokio::test]
async fn custom_broadcast_does_not_invoke_hook_or_respond() {
    let store = CustomStore::new(CustomBehavior::Fixed(vec![0xFF]));

    assert_eq!(respond(&store, &[CUSTOM_FC, 0x01], UnitId(0)).await, None);
    assert_eq!(store.calls(), 0);
}

#[tokio::test]
async fn standard_request_does_not_invoke_custom_hook() {
    let store = CustomStore::new(CustomBehavior::Fixed(vec![0xFF]));

    assert_eq!(
        respond(&store, &[0x03, 0x00, 0x00, 0x00, 0x01], UNIT).await,
        Some(vec![0x03, 0x02, 0x00, 0x00])
    );
    assert_eq!(store.calls(), 0);
}

#[tokio::test]
async fn concurrent_calls_use_independent_response_buffers() {
    const CALLS: usize = 16;

    let store = CustomStore::new(CustomBehavior::ConcurrentEcho(Arc::new(Barrier::new(
        CALLS,
    ))));
    let device_id = DeviceIdentification::default();
    let requests: Vec<Vec<u8>> = (0..CALLS)
        .map(|value| vec![CUSTOM_FC, value as u8, !(value as u8)])
        .collect();
    let responses = join_all(
        requests
            .iter()
            .map(|request| handler::process_request(request, UNIT, &store, &device_id)),
    )
    .await;

    for (request, response) in requests.iter().zip(responses) {
        assert_eq!(response.as_deref(), Some(request.as_slice()));
    }
    let observations = store.observations();
    assert_eq!(observations.len(), CALLS);
    assert!(
        observations
            .iter()
            .all(|observation| observation.response_capacity == MAX_PDU_SIZE - 1)
    );
    assert_eq!(
        observations
            .iter()
            .map(|observation| observation.response_address)
            .collect::<HashSet<_>>()
            .len(),
        CALLS
    );
}

#[tokio::test]
async fn tcp_response_preserves_mbap_identity() {
    let store = Arc::new(CustomStore::new(CustomBehavior::Fixed(vec![0xDE, 0xAD])));
    let server = ModbusServer::start(server_config(), Arc::clone(&store))
        .await
        .unwrap();
    let (mut sink, mut stream) = connect(server.local_addr()).await;
    sink.send(Frame {
        header: FrameHeader::Mbap(MbapHeader::new(0xBEEF, UNIT.0, 3)),
        pdu: Bytes::from_static(&[CUSTOM_FC, 0x12, 0x34]),
    })
    .await
    .unwrap();

    let response = stream.recv().await.unwrap();
    let FrameHeader::Mbap(header) = response.header else {
        panic!("expected MBAP response");
    };
    assert_eq!(header.transaction_id.get(), 0xBEEF);
    assert_eq!(header.unit_id, UNIT.0);
    assert_eq!(header.pdu_length(), 3);
    assert_eq!(response.pdu.as_ref(), &[CUSTOM_FC, 0xDE, 0xAD]);

    drop((sink, stream));
    assert_eq!(server.stop().await, ShutdownOutcome::Drained);
}

#[tokio::test]
async fn custom_hook_panic_reclaims_connection_and_request() {
    let store = Arc::new(CustomStore::new(CustomBehavior::Panic));
    let server = ModbusServer::start(server_config(), Arc::clone(&store))
        .await
        .unwrap();
    let (mut sink, mut stream) = connect(server.local_addr()).await;
    sink.send(Frame {
        header: FrameHeader::Mbap(MbapHeader::new(1, UNIT.0, 1)),
        pdu: Bytes::from_static(&[CUSTOM_FC]),
    })
    .await
    .unwrap();

    assert!(matches!(
        stream.recv().await,
        Err(TransportError::Disconnected)
    ));
    wait_until(|| {
        let metrics = server.metrics();
        metrics.active_connections == 0 && metrics.active_requests == 0
    })
    .await;
    assert_eq!(store.calls(), 1);
    assert_eq!(server.stop().await, ShutdownOutcome::Drained);
}
