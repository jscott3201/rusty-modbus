//! Retiring and verdict-gated pool-to-client handoff integration tests.

#![cfg(feature = "client")]

use std::future::{Future, pending, poll_fn};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_client::{SessionRetirementReason, SessionReuseVerdict};
use rusty_modbus_frame::{Frame, FrameHeader};
use rusty_modbus_pool::{
    ClientConfig, ClientError, ConnectionPool, LeaseInvalidationReason, PoolConfig, PoolError,
    PooledClientReturnOutcome, PriorityDevice, RetryConfig,
};
use rusty_modbus_tcp::config::{TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{MbapHeader, UnitId};
use tokio::sync::{Barrier, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber, dispatcher};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{Layer, registry};

const COMPLETION_TARGET: &str = "rusty_modbus_pool::client_handoff";
const COMPLETION_MESSAGE: &str = "pooled_client_session_completed";

#[derive(Debug, PartialEq, Eq)]
struct CapturedCompletion {
    level: Level,
    message: String,
    outcome: String,
    trigger: String,
    verdict: String,
    retirement_reason: String,
    is_priority: bool,
}

#[derive(Default)]
struct CompletionVisitor {
    message: Option<String>,
    outcome: Option<String>,
    trigger: Option<String>,
    verdict: Option<String>,
    retirement_reason: Option<String>,
    is_priority: Option<bool>,
}

impl Visit for CompletionVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        let slot = match field.name() {
            "outcome" => &mut self.outcome,
            "trigger" => &mut self.trigger,
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
struct CompletionCapture {
    events: Arc<Mutex<Vec<CapturedCompletion>>>,
}

impl CompletionCapture {
    fn dispatch(&self) -> tracing::Dispatch {
        tracing::Dispatch::new(registry().with(self.clone()))
    }

    fn assert_events(&self, expected: &[CapturedCompletion]) {
        assert_eq!(self.events.lock().unwrap().as_slice(), expected);
    }
}

impl<S> Layer<S> for CompletionCapture
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event.metadata().target() != COMPLETION_TARGET {
            return;
        }

        let mut visitor = CompletionVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(CapturedCompletion {
            level: *event.metadata().level(),
            message: visitor.message.expect("completion message field"),
            outcome: visitor.outcome.expect("completion outcome field"),
            trigger: visitor.trigger.expect("completion trigger field"),
            verdict: visitor.verdict.expect("completion verdict field"),
            retirement_reason: visitor
                .retirement_reason
                .expect("completion retirement reason field"),
            is_priority: visitor.is_priority.expect("completion priority field"),
        });
    }
}

fn completion(
    level: Level,
    outcome: &str,
    trigger: &str,
    verdict: &str,
    retirement_reason: &str,
    is_priority: bool,
) -> CapturedCompletion {
    CapturedCompletion {
        level,
        message: COMPLETION_MESSAGE.to_owned(),
        outcome: outcome.to_owned(),
        trigger: trigger.to_owned(),
        verdict: verdict.to_owned(),
        retirement_reason: retirement_reason.to_owned(),
        is_priority,
    }
}

async fn with_capture<F>(capture: &CompletionCapture, future: F) -> F::Output
where
    F: Future,
{
    let dispatch = capture.dispatch();
    let mut future = std::pin::pin!(future);
    poll_fn(|context| dispatcher::with_default(&dispatch, || future.as_mut().poll(context))).await
}

fn drop_with_capture<T>(capture: &CompletionCapture, value: T) {
    let dispatch = capture.dispatch();
    dispatcher::with_default(&dispatch, || drop(value));
}

#[derive(Debug, PartialEq, Eq)]
enum ServerEvent {
    Accepted(u8),
    Request { borrower: u8, transaction_id: u16 },
    HostileFrameSent,
    LateResponseAttempted,
}

fn pool_config() -> PoolConfig {
    PoolConfig {
        max_connections: 1,
        pre_connect: false,
        idle_timeout: Duration::from_secs(300),
        health_check_interval: Duration::from_secs(300),
        tcp_config: TcpConfig {
            read_timeout: None,
            write_timeout: None,
            ..TcpConfig::default()
        },
        ..PoolConfig::default()
    }
}

fn client_config(timeout: Duration) -> ClientConfig {
    ClientConfig {
        timeout,
        shutdown_timeout: Duration::from_secs(1),
        retry: RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        },
        ..ClientConfig::default()
    }
}

fn transaction_id(frame: &Frame) -> u16 {
    match frame.header {
        FrameHeader::Mbap(header) => header.transaction_id.get(),
        FrameHeader::Rtu { .. } => panic!("expected a Modbus/TCP frame"),
    }
}

fn register_response(request: &Frame, value: u16) -> Frame {
    let pdu = Bytes::from(vec![0x03, 0x02, (value >> 8) as u8, value as u8]);
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(
            transaction_id(request),
            request.unit_id(),
            u16::try_from(pdu.len()).unwrap(),
        )),
        pdu,
    }
}

fn response_with_pdu(request: &Frame, pdu: Bytes) -> Frame {
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(
            transaction_id(request),
            request.unit_id(),
            u16::try_from(pdu.len()).unwrap(),
        )),
        pdu,
    }
}

async fn expect_event(events: &mut mpsc::UnboundedReceiver<ServerEvent>, expected: ServerEvent) {
    let actual = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("server event should arrive promptly")
        .expect("server should remain available");
    assert_eq!(actual, expected);
}

async fn isolation_server() -> (
    SocketAddr,
    mpsc::UnboundedReceiver<ServerEvent>,
    oneshot::Sender<()>,
    JoinHandle<()>,
) {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (release_late_tx, release_late_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        let (mut sink_a, mut stream_a, _, _guard_a) = listener.accept().await.unwrap();
        event_tx.send(ServerEvent::Accepted(1)).unwrap();
        let request_a = stream_a.recv().await.unwrap();
        event_tx
            .send(ServerEvent::Request {
                borrower: 1,
                transaction_id: transaction_id(&request_a),
            })
            .unwrap();

        let (mut sink_b, mut stream_b, _, _guard_b) = listener.accept().await.unwrap();
        event_tx.send(ServerEvent::Accepted(2)).unwrap();
        let request_b = stream_b.recv().await.unwrap();
        event_tx
            .send(ServerEvent::Request {
                borrower: 2,
                transaction_id: transaction_id(&request_b),
            })
            .unwrap();

        release_late_rx.await.unwrap();
        let _ = sink_a.send(register_response(&request_a, 0xA001)).await;
        event_tx.send(ServerEvent::LateResponseAttempted).unwrap();
        sink_b
            .send(register_response(&request_b, 0xB002))
            .await
            .unwrap();

        let (_sink_c, _stream_c, _, _guard_c) = listener.accept().await.unwrap();
        event_tx.send(ServerEvent::Accepted(3)).unwrap();
    });

    (addr, event_rx, release_late_tx, task)
}

async fn withholding_server() -> (
    SocketAddr,
    mpsc::UnboundedReceiver<ServerEvent>,
    JoinHandle<()>,
) {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    let task = tokio::spawn(async move {
        let mut borrower = 0u8;
        loop {
            let (_sink, mut stream, _, _guard) = listener.accept().await.unwrap();
            borrower = borrower.checked_add(1).unwrap();
            event_tx.send(ServerEvent::Accepted(borrower)).unwrap();
            let connection_events = event_tx.clone();
            tokio::spawn(async move {
                if let Ok(request) = stream.recv().await {
                    connection_events
                        .send(ServerEvent::Request {
                            borrower,
                            transaction_id: transaction_id(&request),
                        })
                        .unwrap();
                }
                pending::<()>().await;
            });
        }
    });

    (addr, event_rx, task)
}

async fn two_request_reuse_server() -> (
    SocketAddr,
    mpsc::UnboundedReceiver<ServerEvent>,
    oneshot::Sender<()>,
    JoinHandle<()>,
) {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (finish_tx, finish_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        let (mut sink, mut stream, _, _guard) = listener.accept().await.unwrap();
        event_tx.send(ServerEvent::Accepted(1)).unwrap();

        let first = stream.recv().await.unwrap();
        event_tx
            .send(ServerEvent::Request {
                borrower: 1,
                transaction_id: transaction_id(&first),
            })
            .unwrap();
        sink.send(register_response(&first, 0x1001)).await.unwrap();

        tokio::select! {
            second = stream.recv() => {
                let second = second.unwrap();
                event_tx
                    .send(ServerEvent::Request {
                        borrower: 1,
                        transaction_id: transaction_id(&second),
                    })
                    .unwrap();
                sink.send(register_response(&second, 0x2002)).await.unwrap();
            }
            accepted = listener.accept() => {
                let (mut second_sink, mut second_stream, _, _second_guard) = accepted.unwrap();
                event_tx.send(ServerEvent::Accepted(2)).unwrap();
                let second = second_stream.recv().await.unwrap();
                event_tx
                    .send(ServerEvent::Request {
                        borrower: 2,
                        transaction_id: transaction_id(&second),
                    })
                    .unwrap();
                second_sink
                    .send(register_response(&second, 0x2002))
                    .await
                    .unwrap();
            }
        }

        finish_rx.await.unwrap();
    });

    (addr, event_rx, finish_tx, task)
}

#[derive(Clone, Copy)]
enum HostileBehavior {
    UnknownResponse,
    TypedInvalidResponse,
}

async fn hostile_then_fresh_server(
    behavior: HostileBehavior,
) -> (
    SocketAddr,
    mpsc::UnboundedReceiver<ServerEvent>,
    JoinHandle<()>,
) {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    let task = tokio::spawn(async move {
        let (mut sink, mut stream, _, _guard) = listener.accept().await.unwrap();
        event_tx.send(ServerEvent::Accepted(1)).unwrap();
        match behavior {
            HostileBehavior::UnknownResponse => {
                sink.send(Frame {
                    header: FrameHeader::Mbap(MbapHeader::new(41, 1, 4)),
                    pdu: Bytes::from_static(&[0x03, 0x02, 0x00, 0x2A]),
                })
                .await
                .unwrap();
            }
            HostileBehavior::TypedInvalidResponse => {
                let request = stream.recv().await.unwrap();
                event_tx
                    .send(ServerEvent::Request {
                        borrower: 1,
                        transaction_id: transaction_id(&request),
                    })
                    .unwrap();
                sink.send(response_with_pdu(
                    &request,
                    Bytes::from_static(&[0x01, 0x01, 0xFF]),
                ))
                .await
                .unwrap();
            }
        }
        event_tx.send(ServerEvent::HostileFrameSent).unwrap();

        let (_fresh_sink, _fresh_stream, _, _fresh_guard) = listener.accept().await.unwrap();
        event_tx.send(ServerEvent::Accepted(2)).unwrap();
        pending::<()>().await;
    });

    (addr, event_rx, task)
}

async fn single_response_server() -> (
    SocketAddr,
    mpsc::UnboundedReceiver<ServerEvent>,
    oneshot::Sender<()>,
    JoinHandle<()>,
) {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (finish_tx, finish_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        let (mut sink, mut stream, _, _guard) = listener.accept().await.unwrap();
        event_tx.send(ServerEvent::Accepted(1)).unwrap();
        let request = stream.recv().await.unwrap();
        event_tx
            .send(ServerEvent::Request {
                borrower: 1,
                transaction_id: transaction_id(&request),
            })
            .unwrap();
        sink.send(register_response(&request, 0x3003))
            .await
            .unwrap();
        finish_rx.await.unwrap();
    });

    (addr, event_rx, finish_tx, task)
}

async fn acquire_fresh_and_retire(
    pool: &ConnectionPool,
    addr: SocketAddr,
    events: &mut mpsc::UnboundedReceiver<ServerEvent>,
) {
    let mut fresh = pool
        .get_with_acquisition_timeout(addr, Duration::from_secs(1))
        .await
        .unwrap();
    expect_event(events, ServerEvent::Accepted(2)).await;
    fresh.invalidate(LeaseInvalidationReason::CallerDirected);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn healthy_reusable_sessions_reuse_one_accept_wake_waiter_and_keep_exact_accounting() {
    let (addr, mut events, finish, server) = two_request_reuse_server().await;
    let pool = Arc::new(ConnectionPool::new(pool_config()));

    let first = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    assert_eq!(
        first
            .client()
            .read_holding_registers(UnitId(1), 0, 1)
            .await
            .unwrap(),
        vec![0x1001]
    );
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;
    let completion_capture = CompletionCapture::default();
    assert_eq!(
        with_capture(&completion_capture, first.shutdown_and_return()).await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    completion_capture.assert_events(&[completion(
        Level::DEBUG,
        "returned_to_idle",
        "shutdown_and_return",
        "reuse_eligible",
        "none",
        false,
    )]);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 1);

    let second = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    assert_eq!(
        second
            .client()
            .read_holding_registers(UnitId(1), 0, 1)
            .await
            .unwrap(),
        vec![0x2002]
    );
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;

    let waiter_pool = Arc::clone(&pool);
    let waiter = tokio::spawn(async move {
        waiter_pool
            .get_with_acquisition_timeout(addr, Duration::from_secs(1))
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !waiter.is_finished(),
        "bounded waiter should need the active charge"
    );

    assert_eq!(
        second.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    let third = waiter.await.unwrap().unwrap();
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    drop(third);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 1);

    finish.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn reusable_timeout_retires_and_forces_a_fresh_accept() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_millis(50)));

    let error = session
        .client()
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::RetriesExhausted { .. }));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;
    let completion_capture = CompletionCapture::default();
    assert_eq!(
        with_capture(&completion_capture, session.shutdown_and_return()).await,
        PooledClientReturnOutcome::Retired(SessionReuseVerdict::Retire(
            SessionRetirementReason::RequestTimedOut
        ))
    );
    completion_capture.assert_events(&[completion(
        Level::DEBUG,
        "retired",
        "shutdown_and_return",
        "retire",
        "request_timed_out",
        false,
    )]);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn reusable_post_dispatch_cancellation_retires_and_forces_a_fresh_accept() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let session = Arc::new(
        pool.get(addr)
            .await
            .unwrap()
            .into_reusable_client(client_config(Duration::from_secs(30))),
    );

    let request_session = Arc::clone(&session);
    let request = tokio::spawn(async move {
        request_session
            .client()
            .read_holding_registers(UnitId(1), 0, 1)
            .await
    });
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());

    let session = match Arc::try_unwrap(session) {
        Ok(session) => session,
        Err(_) => panic!("request task should release its session owner"),
    };
    assert_eq!(
        session.shutdown_and_return().await,
        PooledClientReturnOutcome::Retired(SessionReuseVerdict::Retire(
            SessionRetirementReason::DispatchCancelled
        ))
    );
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn observed_unknown_response_retires_and_forces_a_fresh_accept() {
    let (addr, mut events, server) =
        hostile_then_fresh_server(HostileBehavior::UnknownResponse).await;
    let pool = ConnectionPool::new(pool_config());
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    expect_event(&mut events, ServerEvent::HostileFrameSent).await;

    let expected = SessionReuseVerdict::Retire(SessionRetirementReason::UnknownOrDuplicateResponse);
    tokio::time::timeout(Duration::from_secs(1), async {
        while session.client().session_reuse_verdict() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("reader should observe the unknown response before shutdown");
    assert_eq!(
        session.shutdown_and_return().await,
        PooledClientReturnOutcome::Retired(expected)
    );
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn typed_validation_failure_retires_and_forces_a_fresh_accept() {
    let (addr, mut events, server) =
        hostile_then_fresh_server(HostileBehavior::TypedInvalidResponse).await;
    let pool = ConnectionPool::new(pool_config());
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));

    assert!(matches!(
        session.client().read_coils(UnitId(1), 0, 64).await,
        Err(ClientError::ShortResponse { .. })
    ));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;
    expect_event(&mut events, ServerEvent::HostileFrameSent).await;
    assert_eq!(
        session.shutdown_and_return().await,
        PooledClientReturnOutcome::Retired(SessionReuseVerdict::Retire(
            SessionRetirementReason::TypedResponseDataInvalid
        ))
    );
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn explicit_abort_retires_and_forces_a_fresh_accept() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;

    session.client().abort();
    assert_eq!(
        session.shutdown_and_return().await,
        PooledClientReturnOutcome::Retired(SessionReuseVerdict::Retire(
            SessionRetirementReason::Aborted
        ))
    );
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn shutdown_deadline_retires_and_forces_a_fresh_accept() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let mut config = client_config(Duration::from_secs(30));
    config.shutdown_timeout = Duration::from_millis(50);
    let session = Arc::new(pool.get(addr).await.unwrap().into_reusable_client(config));

    let request_session = Arc::clone(&session);
    let request = tokio::spawn(async move {
        request_session
            .client()
            .read_holding_registers(UnitId(1), 0, 1)
            .await
    });
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;

    session.client().shutdown().await;
    assert!(request.await.unwrap().is_err());
    let session = match Arc::try_unwrap(session) {
        Ok(session) => session,
        Err(_) => panic!("request task should release its session owner"),
    };
    assert_eq!(
        session.shutdown_and_return().await,
        PooledClientReturnOutcome::Retired(SessionReuseVerdict::Retire(
            SessionRetirementReason::ShutdownDeadlineExceeded
        ))
    );
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn dropping_reusable_session_without_return_retires_and_releases_capacity() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    assert_eq!(pool.active_count(), 1);

    let completion_capture = CompletionCapture::default();
    drop_with_capture(&completion_capture, session);
    completion_capture.assert_events(&[completion(
        Level::DEBUG,
        "retired",
        "wrapper_drop",
        "not_quiescent",
        "none",
        false,
    )]);
    tokio::time::timeout(Duration::from_secs(1), async {
        while pool.active_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("aborted reader task should release the final vault owner");
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn clean_client_shutdown_then_wrapper_drop_retires_with_eligible_verdict() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;

    session.client().shutdown().await;
    assert_eq!(
        session.client().session_reuse_verdict(),
        SessionReuseVerdict::ReuseEligible
    );
    let completion_capture = CompletionCapture::default();
    drop_with_capture(&completion_capture, session);
    completion_capture.assert_events(&[completion(
        Level::DEBUG,
        "retired",
        "wrapper_drop",
        "reuse_eligible",
        "none",
        false,
    )]);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn cancelling_consuming_return_future_emits_once_and_retires_exact_capacity() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    assert_eq!(pool.active_count(), 1);

    let completion_capture = CompletionCapture::default();
    let dispatch = completion_capture.dispatch();
    let mut returning = Box::pin(session.shutdown_and_return());
    let first_poll = poll_fn(|context| {
        Poll::Ready(dispatcher::with_default(&dispatch, || {
            returning.as_mut().poll(context)
        }))
    })
    .await;
    assert!(
        first_poll.is_pending(),
        "shutdown must yield to its spawned coordinator"
    );
    dispatcher::with_default(&dispatch, || drop(returning));
    completion_capture.assert_events(&[completion(
        Level::DEBUG,
        "retired",
        "wrapper_drop",
        "not_quiescent",
        "none",
        false,
    )]);

    tokio::time::timeout(Duration::from_secs(1), async {
        while pool.active_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled return should release its exact active charge");
    assert_eq!(pool.idle_count(), 0);

    acquire_fresh_and_retire(&pool, addr, &mut events).await;
    server.abort();
}

#[tokio::test]
async fn pool_shutdown_before_reusable_return_wins_without_idle_resurrection() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;

    pool.shutdown();
    let completion_capture = CompletionCapture::default();
    assert_eq!(
        with_capture(&completion_capture, session.shutdown_and_return()).await,
        PooledClientReturnOutcome::PoolShuttingDown
    );
    completion_capture.assert_events(&[completion(
        Level::DEBUG,
        "pool_shutting_down",
        "shutdown_and_return",
        "reuse_eligible",
        "none",
        false,
    )]);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
    assert!(matches!(pool.get(addr).await, Err(PoolError::ShuttingDown)));
    server.abort();
}

#[tokio::test]
async fn concurrent_pool_shutdown_and_reusable_return_release_once_without_idle_resurrection() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = Arc::new(ConnectionPool::new(pool_config()));
    let session = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;

    let barrier = Arc::new(Barrier::new(3));
    let return_barrier = Arc::clone(&barrier);
    let returning = tokio::spawn(async move {
        return_barrier.wait().await;
        session.shutdown_and_return().await
    });
    let shutdown_pool = Arc::clone(&pool);
    let shutdown_barrier = Arc::clone(&barrier);
    let shutting_down = tokio::spawn(async move {
        shutdown_barrier.wait().await;
        shutdown_pool.shutdown();
    });
    barrier.wait().await;

    let outcome = returning.await.unwrap();
    shutting_down.await.unwrap();
    assert!(matches!(
        outcome,
        PooledClientReturnOutcome::ReturnedToIdle | PooledClientReturnOutcome::PoolShuttingDown
    ));
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
    pool.shutdown();
    assert_eq!(pool.active_count(), 0);
    server.abort();
}

#[tokio::test]
async fn priority_reusable_return_preserves_separate_budget_and_exact_active_charge() {
    let (priority_addr, mut priority_events, finish, priority_server) =
        single_response_server().await;
    let (ordinary_addr, mut ordinary_events, ordinary_server) = withholding_server().await;
    let mut config = pool_config();
    config.priority_devices = vec![PriorityDevice {
        addr: priority_addr,
        max_connections: 1,
    }];
    let pool = ConnectionPool::new(config);

    let priority = pool
        .get(priority_addr)
        .await
        .unwrap()
        .into_reusable_client(client_config(Duration::from_secs(1)));
    expect_event(&mut priority_events, ServerEvent::Accepted(1)).await;
    let mut ordinary = pool.get(ordinary_addr).await.unwrap();
    expect_event(&mut ordinary_events, ServerEvent::Accepted(1)).await;
    assert_eq!(
        pool.active_count(),
        2,
        "priority activity must not consume the non-priority budget"
    );

    assert_eq!(
        priority
            .client()
            .read_holding_registers(UnitId(1), 0, 1)
            .await
            .unwrap(),
        vec![0x3003]
    );
    expect_event(
        &mut priority_events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;
    let completion_capture = CompletionCapture::default();
    assert_eq!(
        with_capture(&completion_capture, priority.shutdown_and_return()).await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    completion_capture.assert_events(&[completion(
        Level::DEBUG,
        "returned_to_idle",
        "shutdown_and_return",
        "reuse_eligible",
        "none",
        true,
    )]);
    assert_eq!(
        pool.active_count(),
        1,
        "only the ordinary lease remains active"
    );
    assert_eq!(pool.idle_count(), 1);

    ordinary.invalidate(LeaseInvalidationReason::CallerDirected);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 1);

    let priority_again = pool.get(priority_addr).await.unwrap();
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    drop(priority_again);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 1);

    finish.send(()).unwrap();
    priority_server.await.unwrap();
    ordinary_server.abort();
}

#[tokio::test]
async fn timed_out_late_response_cannot_cross_borrowers_and_healthy_sessions_retire() {
    let (addr, mut events, release_late, server) = isolation_server().await;
    let pool = ConnectionPool::new(pool_config());

    let lease_a = pool.get(addr).await.unwrap();
    let client_a = lease_a.into_retiring_client(client_config(Duration::from_millis(100)));
    let error = client_a
        .read_holding_registers(UnitId(1), 0, 1)
        .await
        .unwrap_err();
    match error {
        ClientError::RetriesExhausted {
            attempts: 1,
            last_error,
        } => assert!(matches!(*last_error, ClientError::Timeout)),
        other => panic!("expected one timed-out attempt, got {other:?}"),
    }
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;

    client_a.shutdown().await;
    assert_eq!(
        client_a.session_reuse_verdict(),
        SessionReuseVerdict::Retire(SessionRetirementReason::RequestTimedOut)
    );
    assert_eq!(
        pool.active_count(),
        1,
        "the surviving sink retains capacity"
    );
    assert_eq!(pool.idle_count(), 0);
    assert!(matches!(
        pool.get_with_acquisition_timeout(addr, Duration::from_millis(20))
            .await,
        Err(PoolError::Timeout)
    ));

    drop(client_a);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let lease_b = pool
        .get_with_acquisition_timeout(addr, Duration::from_secs(1))
        .await
        .unwrap();
    expect_event(&mut events, ServerEvent::Accepted(2)).await;
    let client_b = Arc::new(lease_b.into_retiring_client(client_config(Duration::from_secs(1))));
    let request_client = Arc::clone(&client_b);
    let request_b =
        tokio::spawn(async move { request_client.read_holding_registers(UnitId(1), 0, 1).await });
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 2,
            transaction_id: 1,
        },
    )
    .await;
    release_late.send(()).unwrap();
    expect_event(&mut events, ServerEvent::LateResponseAttempted).await;
    assert_eq!(request_b.await.unwrap().unwrap(), vec![0xB002]);

    client_b.shutdown().await;
    assert_eq!(
        client_b.session_reuse_verdict(),
        SessionReuseVerdict::ReuseEligible
    );
    assert_eq!(
        pool.active_count(),
        1,
        "healthy client sink still owns capacity"
    );
    drop(client_b);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0, "healthy handoffs also retire");

    let lease_c = pool
        .get_with_acquisition_timeout(addr, Duration::from_secs(1))
        .await
        .unwrap();
    expect_event(&mut events, ServerEvent::Accepted(3)).await;
    let client_c = lease_c.into_retiring_client(client_config(Duration::from_secs(1)));
    client_c.shutdown().await;
    drop(client_c);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
    server.await.unwrap();
}

#[tokio::test]
async fn cancelled_request_after_send_never_returns_connection_to_idle() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let lease = pool.get(addr).await.unwrap();
    let client = Arc::new(lease.into_retiring_client(client_config(Duration::from_secs(30))));

    let request_client = Arc::clone(&client);
    let request =
        tokio::spawn(async move { request_client.read_holding_registers(UnitId(1), 0, 1).await });
    expect_event(&mut events, ServerEvent::Accepted(1)).await;
    expect_event(
        &mut events,
        ServerEvent::Request {
            borrower: 1,
            transaction_id: 1,
        },
    )
    .await;

    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    client.shutdown().await;
    assert_eq!(pool.active_count(), 1);
    drop(client);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let mut fresh = pool
        .get_with_acquisition_timeout(addr, Duration::from_secs(1))
        .await
        .unwrap();
    expect_event(&mut events, ServerEvent::Accepted(2)).await;
    fresh.invalidate(LeaseInvalidationReason::CallerDirected);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
    server.abort();
}

#[tokio::test]
async fn pool_shutdown_then_client_teardown_releases_once_without_idle_resurrection() {
    let (addr, mut events, server) = withholding_server().await;
    let pool = ConnectionPool::new(pool_config());
    let client = pool
        .get(addr)
        .await
        .unwrap()
        .into_retiring_client(client_config(Duration::from_secs(1)));
    expect_event(&mut events, ServerEvent::Accepted(1)).await;

    pool.shutdown();
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    client.shutdown().await;
    assert_eq!(pool.active_count(), 1);
    drop(client);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    pool.shutdown();
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
    assert!(matches!(pool.get(addr).await, Err(PoolError::ShuttingDown)));
    server.abort();
}

#[test]
fn runtime_drop_then_reusable_session_drop_retires_without_panicking_or_leaking_capacity() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (pool, session) = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (_socket, _) = listener.accept().await.unwrap();
                pending::<()>().await;
            });

            let pool = ConnectionPool::new(pool_config());
            let session = pool
                .get(addr)
                .await
                .unwrap()
                .into_reusable_client(client_config(Duration::from_secs(1)));
            (pool, session)
        });

        assert_eq!(pool.active_count(), 1);
        drop(runtime);
        assert_eq!(
            pool.active_count(),
            1,
            "the session survives runtime teardown"
        );
        drop(session);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }));

    assert!(result.is_ok());
}
