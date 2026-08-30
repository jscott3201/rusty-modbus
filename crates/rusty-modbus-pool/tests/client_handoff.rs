//! Retiring pool-to-client handoff integration tests.

#![cfg(feature = "client")]

use std::future::pending;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::{Frame, FrameHeader};
use rusty_modbus_pool::{
    ClientConfig, ClientError, ConnectionPool, LeaseInvalidationReason, PoolConfig, PoolError,
    RetryConfig,
};
use rusty_modbus_tcp::config::{TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::{MbapHeader, UnitId};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

#[derive(Debug, PartialEq, Eq)]
enum ServerEvent {
    Accepted(u8),
    Request { borrower: u8, transaction_id: u16 },
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
fn runtime_drop_then_client_drop_retires_without_panicking_or_leaking_capacity() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (pool, client) = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (_socket, _) = listener.accept().await.unwrap();
                pending::<()>().await;
            });

            let pool = ConnectionPool::new(pool_config());
            let client = pool
                .get(addr)
                .await
                .unwrap()
                .into_retiring_client(client_config(Duration::from_secs(1)));
            (pool, client)
        });

        assert_eq!(pool.active_count(), 1);
        drop(runtime);
        assert_eq!(pool.active_count(), 1, "the sink survives runtime teardown");
        drop(client);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }));

    assert!(result.is_ok());
}
