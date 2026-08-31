//! Active priority-device probe integration and lifecycle tests.

#![cfg(feature = "client")]

use std::future::{Future, poll_fn};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::{Frame, FrameHeader};
use rusty_modbus_pool::{
    ClientConfig, ConnectionPool, PoolConfig, PoolError, PooledClientReturnOutcome, PriorityDevice,
    PriorityProbeConfig, PriorityProbeOperation,
};
use rusty_modbus_tcp::config::{TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{Address, MbapHeader, Quantity, UnitId};
use tokio::sync::{mpsc, oneshot};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber, dispatcher};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{Layer, registry};

const PROBE_INTERVAL: Duration = Duration::from_secs(60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
static PROBE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct ProbeRequest {
    connection: u8,
    unit_id: u8,
    pdu: Bytes,
    respond: oneshot::Sender<ProbeResponse>,
}

#[derive(Debug)]
struct ProbeResponse {
    unit_id: u8,
    pdu: Bytes,
}

enum ServerEvent {
    Accepted(u8),
    Request(ProbeRequest),
    Closed(u8),
}

fn transaction_id(frame: &Frame) -> u16 {
    match frame.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected Modbus/TCP frame"),
    }
}

async fn scripted_server() -> (
    SocketAddr,
    mpsc::UnboundedReceiver<ServerEvent>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), server_config())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let (events_tx, events_rx) = mpsc::unbounded_channel();

    let server = tokio::spawn(async move {
        let mut connection = 0_u8;
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            connection = connection.checked_add(1).unwrap();
            let current = connection;
            if events_tx.send(ServerEvent::Accepted(current)).is_err() {
                return;
            }
            let connection_events = events_tx.clone();
            tokio::spawn(async move {
                loop {
                    let request = match stream.recv().await {
                        Ok(request) => request,
                        Err(_) => {
                            let _ = connection_events.send(ServerEvent::Closed(current));
                            return;
                        }
                    };
                    let request_transaction = transaction_id(&request);
                    let (respond_tx, respond_rx) = oneshot::channel();
                    if connection_events
                        .send(ServerEvent::Request(ProbeRequest {
                            connection: current,
                            unit_id: request.unit_id(),
                            pdu: request.pdu,
                            respond: respond_tx,
                        }))
                        .is_err()
                    {
                        return;
                    }

                    let Ok(response) = respond_rx.await else {
                        continue;
                    };
                    let frame = Frame {
                        header: FrameHeader::Mbap(MbapHeader::new(
                            request_transaction,
                            response.unit_id,
                            u16::try_from(response.pdu.len()).unwrap(),
                        )),
                        pdu: response.pdu,
                    };
                    let _ = sink.send(frame).await;
                }
            });
        }
    });

    (addr, events_rx, server)
}

fn server_config() -> TcpServerConfig {
    TcpServerConfig {
        tcp: TcpConfig {
            read_timeout: None,
            write_timeout: None,
            ..TcpConfig::default()
        },
        ..TcpServerConfig::default()
    }
}

#[derive(Clone, Copy)]
struct ProbeOptions {
    operation: PriorityProbeOperation,
    quantity: u16,
    interval: Duration,
    timeout: Duration,
    pre_connect: bool,
    replenishment: bool,
    max_connections: usize,
}

fn probe_options(operation: PriorityProbeOperation) -> ProbeOptions {
    ProbeOptions {
        operation,
        quantity: 1,
        interval: PROBE_INTERVAL,
        timeout: PROBE_TIMEOUT,
        pre_connect: true,
        replenishment: false,
        max_connections: 1,
    }
}

fn probe_config(addr: SocketAddr, options: ProbeOptions) -> PoolConfig {
    PoolConfig {
        pre_connect: options.pre_connect,
        priority_replenishment: options.replenishment,
        priority_devices: vec![PriorityDevice {
            addr,
            max_connections: options.max_connections,
            probe: Some(
                PriorityProbeConfig::new(
                    options.operation,
                    UnitId(7),
                    Address(0x1234),
                    Quantity(options.quantity),
                    options.interval,
                    options.timeout,
                )
                .unwrap(),
            ),
        }],
        health_check_interval: Duration::from_hours(1),
        tcp_config: TcpConfig {
            connect_timeout: Duration::from_hours(1),
            read_timeout: None,
            write_timeout: None,
            ..TcpConfig::default()
        },
        ..PoolConfig::default()
    }
}

async fn next_event(events: &mut mpsc::UnboundedReceiver<ServerEvent>) -> ServerEvent {
    for _ in 0..100_000 {
        if let Ok(event) = events.try_recv() {
            return event;
        }
        tokio::task::yield_now().await;
    }
    panic!("server event should arrive promptly");
}

async fn expect_accepted(events: &mut mpsc::UnboundedReceiver<ServerEvent>, expected: u8) {
    loop {
        match next_event(events).await {
            ServerEvent::Accepted(connection) => {
                assert_eq!(connection, expected);
                return;
            }
            ServerEvent::Closed(_) => {}
            ServerEvent::Request(_) => panic!("unexpected request before connection acceptance"),
        }
    }
}

async fn expect_request(events: &mut mpsc::UnboundedReceiver<ServerEvent>) -> ProbeRequest {
    loop {
        match next_event(events).await {
            ServerEvent::Request(request) => return request,
            ServerEvent::Closed(_) => {}
            ServerEvent::Accepted(connection) => {
                panic!("unexpected connection {connection} before request")
            }
        }
    }
}

async fn expect_closed(events: &mut mpsc::UnboundedReceiver<ServerEvent>, expected: u8) {
    loop {
        match next_event(events).await {
            ServerEvent::Closed(connection) if connection == expected => return,
            ServerEvent::Closed(_) => {}
            ServerEvent::Accepted(connection) => {
                panic!("unexpected connection {connection} while waiting for closure")
            }
            ServerEvent::Request(_) => panic!("unexpected request while waiting for closure"),
        }
    }
}

async fn wait_for_idle(pool: &ConnectionPool, expected: usize) {
    for _ in 0..100_000 {
        if pool.idle_count() == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!(
        "pool should reach idle count {expected} promptly (active={}, idle={})",
        pool.active_count(),
        pool.idle_count()
    );
}

async fn wait_for_probe_events(capture: &ProbeCapture, expected: usize) {
    for _ in 0..100_000 {
        if capture.events.lock().unwrap().len() == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("probe completion event should be emitted promptly");
}

fn normal_response(function: u8, quantity: u16) -> Bytes {
    match function {
        0x01 | 0x02 => {
            let bytes = usize::from(quantity).div_ceil(8);
            let mut pdu = vec![function, u8::try_from(bytes).unwrap()];
            pdu.resize(2 + bytes, 0x01);
            Bytes::from(pdu)
        }
        0x03 | 0x04 => {
            let mut pdu = vec![function, u8::try_from(quantity * 2).unwrap()];
            for value in 0..quantity {
                pdu.extend_from_slice(&(0x1000 + value).to_be_bytes());
            }
            Bytes::from(pdu)
        }
        _ => panic!("unsupported test function"),
    }
}

fn answer_normally(request: ProbeRequest, quantity: u16) {
    let function = request.pdu[0];
    request
        .respond
        .send(ProbeResponse {
            unit_id: request.unit_id,
            pdu: normal_response(function, quantity),
        })
        .expect("probe should still await its response");
}

#[derive(Debug, PartialEq, Eq)]
struct CapturedProbe {
    message: String,
    operation: String,
    probe_result: String,
    pool_outcome: String,
    verdict: String,
    retirement_reason: String,
    is_priority: bool,
}

#[derive(Default)]
struct ProbeVisitor {
    message: Option<String>,
    operation: Option<String>,
    probe_result: Option<String>,
    pool_outcome: Option<String>,
    verdict: Option<String>,
    retirement_reason: Option<String>,
    is_priority: Option<bool>,
}

impl Visit for ProbeVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        let slot = match field.name() {
            "operation" => &mut self.operation,
            "probe_result" => &mut self.probe_result,
            "pool_outcome" => &mut self.pool_outcome,
            "verdict" => &mut self.verdict,
            "retirement_reason" => &mut self.retirement_reason,
            _ => return,
        };
        *slot = Some(value.to_owned());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "is_priority" {
            self.is_priority = Some(value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

#[derive(Clone, Default)]
struct ProbeCapture {
    events: Arc<Mutex<Vec<CapturedProbe>>>,
}

impl<S> Layer<S> for ProbeCapture
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event.metadata().target() != "rusty_modbus_pool::priority_probe" {
            return;
        }

        let mut visitor = ProbeVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(CapturedProbe {
            message: visitor.message.expect("probe message"),
            operation: visitor.operation.expect("probe operation"),
            probe_result: visitor.probe_result.expect("probe result"),
            pool_outcome: visitor.pool_outcome.expect("pool outcome"),
            verdict: visitor.verdict.expect("probe verdict"),
            retirement_reason: visitor.retirement_reason.expect("retirement reason"),
            is_priority: visitor.is_priority.expect("priority flag"),
        });
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn fc01_through_fc04_each_probe_once_validate_and_reuse_the_same_tcp_connection() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    let capture = ProbeCapture::default();
    let dispatch = tracing::Dispatch::new(registry().with(capture.clone()));
    let _default = dispatcher::set_default(&dispatch);
    let cases = [
        (PriorityProbeOperation::ReadCoils, 3, 0x01),
        (PriorityProbeOperation::ReadDiscreteInputs, 9, 0x02),
        (PriorityProbeOperation::ReadHoldingRegisters, 1, 0x03),
        (PriorityProbeOperation::ReadInputRegisters, 2, 0x04),
    ];

    for (index, (operation, quantity, function)) in cases.into_iter().enumerate() {
        let (addr, mut events, server) = scripted_server().await;
        let pool = ConnectionPool::new(probe_config(
            addr,
            ProbeOptions {
                operation,
                quantity,
                ..probe_options(operation)
            },
        ));
        expect_accepted(&mut events, 1).await;
        wait_for_idle(&pool, 1).await;
        assert!(events.try_recv().is_err());

        tokio::time::advance(PROBE_INTERVAL + Duration::from_millis(1)).await;
        let request = expect_request(&mut events).await;
        assert_eq!(request.connection, 1);
        assert_eq!(request.unit_id, 7);
        assert_eq!(request.pdu[0], function);
        assert_eq!(&request.pdu[1..], &[0x12, 0x34, 0x00, quantity as u8]);
        answer_normally(request, quantity);

        wait_for_idle(&pool, 1).await;
        wait_for_probe_events(&capture, index + 1).await;
        let reused = pool.get(addr).await.unwrap();
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);
        assert!(
            events.try_recv().is_err(),
            "successful probe return must not open or use another connection"
        );
        drop(reused);
        pool.shutdown_and_wait().await;
        server.abort();
    }

    let actual = capture.events.lock().unwrap();
    assert_eq!(actual.len(), 4);
    for (event, (operation, _, _)) in actual.iter().zip(cases) {
        assert_eq!(
            event,
            &CapturedProbe {
                message: "priority_probe_completed".to_owned(),
                operation: operation.as_str().to_owned(),
                probe_result: "success".to_owned(),
                pool_outcome: "returned_to_idle".to_owned(),
                verdict: "reuse_eligible".to_owned(),
                retirement_reason: "none".to_owned(),
                is_priority: true,
            }
        );
    }
}

#[tokio::test(start_paused = true)]
async fn notification_wakes_do_not_postpone_deadline_and_completion_has_no_catch_up_burst() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    let (addr, mut events, server) = scripted_server().await;
    let operation = PriorityProbeOperation::ReadHoldingRegisters;
    let pool = ConnectionPool::new(probe_config(
        addr,
        ProbeOptions {
            timeout: Duration::from_secs(600),
            ..probe_options(operation)
        },
    ));
    expect_accepted(&mut events, 1).await;
    wait_for_idle(&pool, 1).await;

    tokio::time::advance(Duration::from_secs(30)).await;
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    assert_eq!(
        session.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    assert!(events.try_recv().is_err());

    tokio::time::advance(Duration::from_secs(29)).await;
    assert!(events.try_recv().is_err());
    tokio::time::advance(Duration::from_secs(1)).await;
    let first = expect_request(&mut events).await;

    tokio::time::advance(PROBE_INTERVAL * 3).await;
    assert!(
        events.try_recv().is_err(),
        "a blocked probe must serialize all later due instants"
    );
    answer_normally(first, 1);
    wait_for_idle(&pool, 1).await;

    tokio::time::advance(Duration::from_secs(59)).await;
    assert!(events.try_recv().is_err());
    tokio::time::advance(Duration::from_secs(1)).await;
    let second = expect_request(&mut events).await;
    answer_normally(second, 1);
    wait_for_idle(&pool, 1).await;

    pool.shutdown_and_wait().await;
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn probe_only_never_connects_but_probes_a_demand_returned_idle_entry() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    let (addr, mut events, server) = scripted_server().await;
    let operation = PriorityProbeOperation::ReadCoils;
    let pool = ConnectionPool::new(probe_config(
        addr,
        ProbeOptions {
            interval: Duration::from_secs(10),
            pre_connect: false,
            ..probe_options(operation)
        },
    ));

    tokio::time::advance(Duration::from_secs(30)).await;
    assert!(events.try_recv().is_err());
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    expect_accepted(&mut events, 1).await;
    assert_eq!(
        session.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );

    tokio::time::advance(Duration::from_secs(9)).await;
    assert!(events.try_recv().is_err());
    tokio::time::advance(Duration::from_secs(1)).await;
    let request = expect_request(&mut events).await;
    answer_normally(request, 1);
    wait_for_idle(&pool, 1).await;
    assert!(events.try_recv().is_err());

    pool.shutdown_and_wait().await;
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn first_duplicate_owns_probe_and_first_zero_cap_suppresses_all_background_activity() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    let (addr, mut events, server) = scripted_server().await;
    let first = PriorityProbeConfig::new(
        PriorityProbeOperation::ReadCoils,
        UnitId(7),
        Address(0),
        Quantity(1),
        PROBE_INTERVAL,
        PROBE_TIMEOUT,
    )
    .unwrap();
    let second = PriorityProbeConfig::new(
        PriorityProbeOperation::ReadInputRegisters,
        UnitId(7),
        Address(0),
        Quantity(1),
        PROBE_INTERVAL,
        PROBE_TIMEOUT,
    )
    .unwrap();
    let operation = PriorityProbeOperation::ReadHoldingRegisters;
    let mut config = probe_config(
        addr,
        ProbeOptions {
            replenishment: true,
            ..probe_options(operation)
        },
    );
    config.priority_devices = vec![
        PriorityDevice {
            addr,
            max_connections: 1,
            probe: Some(first),
        },
        PriorityDevice {
            addr,
            max_connections: 3,
            probe: Some(second),
        },
    ];
    let pool = ConnectionPool::new(config);
    expect_accepted(&mut events, 1).await;
    wait_for_idle(&pool, 1).await;
    tokio::time::advance(PROBE_INTERVAL).await;
    let request = expect_request(&mut events).await;
    assert_eq!(request.pdu[0], 0x01);
    answer_normally(request, 1);
    wait_for_idle(&pool, 1).await;
    pool.shutdown_and_wait().await;
    server.abort();

    let (addr, mut events, server) = scripted_server().await;
    let operation = PriorityProbeOperation::ReadCoils;
    let mut zero = probe_config(
        addr,
        ProbeOptions {
            replenishment: true,
            max_connections: 0,
            ..probe_options(operation)
        },
    );
    zero.priority_devices.push(PriorityDevice {
        addr,
        max_connections: 1,
        probe: Some(second),
    });
    let pool = ConnectionPool::new(zero);
    tokio::time::advance(PROBE_INTERVAL * 2).await;
    assert!(events.try_recv().is_err());
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
    pool.shutdown_and_wait().await;
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn claimed_probe_excludes_checkout_and_serializes_cap_two_replenishment_connectors() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    let (addr, mut events, server) = scripted_server().await;
    let operation = PriorityProbeOperation::ReadHoldingRegisters;
    let pool = ConnectionPool::new(probe_config(addr, probe_options(operation)));
    expect_accepted(&mut events, 1).await;
    wait_for_idle(&pool, 1).await;
    tokio::time::advance(PROBE_INTERVAL).await;
    let request = expect_request(&mut events).await;
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    assert!(matches!(pool.get(addr).await, Err(PoolError::Exhausted)));
    assert!(events.try_recv().is_err());
    answer_normally(request, 1);
    wait_for_idle(&pool, 1).await;
    pool.shutdown_and_wait().await;
    server.abort();

    let (addr, mut events, server) = scripted_server().await;
    let pool = ConnectionPool::new(probe_config(
        addr,
        ProbeOptions {
            replenishment: true,
            max_connections: 2,
            ..probe_options(operation)
        },
    ));
    expect_accepted(&mut events, 1).await;
    wait_for_idle(&pool, 1).await;
    tokio::time::advance(PROBE_INTERVAL).await;
    let request = expect_request(&mut events).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(
        events.try_recv().is_err(),
        "the integrated maintainer cannot connect while its probe is blocked"
    );
    answer_normally(request, 1);
    wait_for_idle(&pool, 1).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(
        events.try_recv().is_err(),
        "successful returned idle must satisfy standing maintenance"
    );
    pool.shutdown_and_wait().await;
    server.abort();
}

#[derive(Clone, Copy)]
enum HostileResponse {
    Exception,
    WrongUnit,
    Malformed,
}

#[tokio::test(start_paused = true)]
async fn exception_wrong_unit_and_malformed_probe_responses_retire_and_replenish() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    for behavior in [
        HostileResponse::Exception,
        HostileResponse::WrongUnit,
        HostileResponse::Malformed,
    ] {
        let (addr, mut events, server) = scripted_server().await;
        let operation = PriorityProbeOperation::ReadHoldingRegisters;
        let pool = ConnectionPool::new(probe_config(
            addr,
            ProbeOptions {
                replenishment: true,
                ..probe_options(operation)
            },
        ));
        expect_accepted(&mut events, 1).await;
        wait_for_idle(&pool, 1).await;
        tokio::time::advance(PROBE_INTERVAL).await;
        let request = expect_request(&mut events).await;
        let response = match behavior {
            HostileResponse::Exception => ProbeResponse {
                unit_id: request.unit_id,
                pdu: Bytes::from_static(&[0x83, 0x02]),
            },
            HostileResponse::WrongUnit => ProbeResponse {
                unit_id: request.unit_id + 1,
                pdu: normal_response(0x03, 1),
            },
            HostileResponse::Malformed => ProbeResponse {
                unit_id: request.unit_id,
                pdu: Bytes::from_static(&[0x03, 0x01, 0xff]),
            },
        };
        request
            .respond
            .send(response)
            .expect("probe should still await hostile response");

        expect_accepted(&mut events, 2).await;
        wait_for_idle(&pool, 1).await;
        assert_eq!(pool.active_count(), 0);
        let replacement = pool.get(addr).await.unwrap();
        assert!(events.try_recv().is_err());
        drop(replacement);
        pool.shutdown_and_wait().await;
        server.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_late_response_retires_old_transport_and_wakes_replenishment() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    let (addr, mut events, server) = scripted_server().await;
    let operation = PriorityProbeOperation::ReadHoldingRegisters;
    let pool = ConnectionPool::new(probe_config(
        addr,
        ProbeOptions {
            timeout: Duration::from_secs(5),
            replenishment: true,
            ..probe_options(operation)
        },
    ));
    expect_accepted(&mut events, 1).await;
    wait_for_idle(&pool, 1).await;
    tokio::time::advance(PROBE_INTERVAL).await;
    let timed_out = expect_request(&mut events).await;

    tokio::time::advance(Duration::from_secs(5)).await;
    expect_accepted(&mut events, 2).await;
    wait_for_idle(&pool, 1).await;
    timed_out
        .respond
        .send(ProbeResponse {
            unit_id: timed_out.unit_id,
            pdu: normal_response(0x03, 1),
        })
        .expect("server request task should retain the late response channel");
    expect_closed(&mut events, 1).await;

    let replacement = pool.get(addr).await.unwrap();
    assert!(events.try_recv().is_err());
    drop(replacement);
    pool.shutdown_and_wait().await;
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn blocked_probe_sync_shutdown_and_cancelled_first_waiter_still_join_full_cleanup() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    let (addr, mut events, server) = scripted_server().await;
    let operation = PriorityProbeOperation::ReadHoldingRegisters;
    let pool = ConnectionPool::new(probe_config(
        addr,
        ProbeOptions {
            timeout: Duration::from_secs(600),
            ..probe_options(operation)
        },
    ));
    expect_accepted(&mut events, 1).await;
    wait_for_idle(&pool, 1).await;
    tokio::time::advance(PROBE_INTERVAL).await;
    let blocked = expect_request(&mut events).await;
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    pool.shutdown();
    let mut first_waiter = Box::pin(pool.shutdown_and_wait());
    let first_poll = poll_fn(|context| Poll::Ready(first_waiter.as_mut().poll(context))).await;
    assert!(first_poll.is_pending());
    drop(first_waiter);

    pool.shutdown_and_wait().await;
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
    assert!(matches!(pool.get(addr).await, Err(PoolError::ShuttingDown)));
    pool.shutdown_and_wait().await;

    drop(blocked.respond);
    expect_closed(&mut events, 1).await;
    server.abort();
}

#[tokio::test(start_paused = true)]
async fn passive_adverse_idle_entry_retires_before_probe_sends_any_request() {
    let _test_lock = PROBE_TEST_LOCK.lock().await;
    let listener = TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), server_config())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let (accepted_tx, mut accepted_rx) = oneshot::channel();
    let (observed_tx, observed_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut sink, mut stream, _, _guard) = listener.accept().await.unwrap();
        accepted_tx.send(()).unwrap();
        sink.send(Frame {
            header: FrameHeader::Mbap(MbapHeader::new(999, 7, 4)),
            pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x01]),
        })
        .await
        .unwrap();
        let observed = stream.recv().await;
        let _ = observed_tx.send(observed);
    });
    let pool = ConnectionPool::new(probe_config(
        addr,
        probe_options(PriorityProbeOperation::ReadHoldingRegisters),
    ));
    let mut accepted = false;
    for _ in 0..100_000 {
        if accepted_rx.try_recv().is_ok() {
            accepted = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(accepted, "server should accept the pre-connected transport");
    wait_for_idle(&pool, 1).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }

    tokio::time::advance(PROBE_INTERVAL).await;
    wait_for_idle(&pool, 0).await;
    let observed = observed_rx.await.unwrap();
    assert!(
        observed.is_err(),
        "passive retirement must close before any active request"
    );
    assert_eq!(pool.active_count(), 0);

    pool.shutdown_and_wait().await;
    server.await.unwrap();
}
