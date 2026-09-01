//! Integration tests for the connection pool.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_pool::{
    BackoffConfig, ConnectionPool, LeaseInvalidationReason, PoolConfig, PoolError,
    PoolMetricsSnapshot, PriorityDevice,
};
#[cfg(feature = "client")]
use rusty_modbus_pool::{ClientConfig, PooledClientReturnOutcome};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::config::{TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::MbapHeader;
use tokio::sync::{mpsc, oneshot};

/// Start an echo server on an ephemeral port.
async fn echo_server() -> SocketAddr {
    let config = TcpServerConfig::default();
    let listener = TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            tokio::spawn(async move {
                while let Ok(frame) = stream.recv().await {
                    if sink.send(frame).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    addr
}

/// Start an echo server that reports every accepted TCP connection.
async fn tracked_echo_server() -> (SocketAddr, mpsc::UnboundedReceiver<()>) {
    let config = TcpServerConfig::default();
    let listener = TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), config)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            if accepted_tx.send(()).is_err() {
                return;
            }
            tokio::spawn(async move {
                while let Ok(frame) = stream.recv().await {
                    if sink.send(frame).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    (addr, accepted_rx)
}

async fn wait_for_accept(accepted: &mut mpsc::UnboundedReceiver<()>) {
    tokio::time::timeout(Duration::from_secs(1), accepted.recv())
        .await
        .expect("server should accept a fresh connection promptly")
        .expect("tracked server should remain available");
}

async fn wait_for_idle_count(pool: &ConnectionPool, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while pool.idle_count() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pool should reach the expected idle count promptly");
}

#[derive(Debug, PartialEq, Eq)]
enum RawIsolationEvent {
    Accepted(u8),
    Request(u8),
    LateResponseAttempted,
}

async fn raw_late_response_server() -> (
    SocketAddr,
    mpsc::UnboundedReceiver<RawIsolationEvent>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener =
        TcpServerListener::bind("127.0.0.1:0".parse().unwrap(), TcpServerConfig::default())
            .await
            .unwrap();
    let addr = listener.local_addr().unwrap();
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (release_late_tx, release_late_rx) = oneshot::channel();

    let task = tokio::spawn(async move {
        let (mut sink_a, mut stream_a, _, _guard_a) = listener.accept().await.unwrap();
        events_tx.send(RawIsolationEvent::Accepted(1)).unwrap();
        let request_a = stream_a.recv().await.unwrap();
        events_tx.send(RawIsolationEvent::Request(1)).unwrap();

        let (mut sink_b, mut stream_b, _, _guard_b) = listener.accept().await.unwrap();
        events_tx.send(RawIsolationEvent::Accepted(2)).unwrap();
        let request_b = stream_b.recv().await.unwrap();
        events_tx.send(RawIsolationEvent::Request(2)).unwrap();

        release_late_rx.await.unwrap();
        let _ = sink_a.send(request_a).await;
        events_tx
            .send(RawIsolationEvent::LateResponseAttempted)
            .unwrap();
        sink_b.send(request_b).await.unwrap();
    });

    (addr, events_rx, release_late_tx, task)
}

async fn expect_raw_event(
    events: &mut mpsc::UnboundedReceiver<RawIsolationEvent>,
    expected: RawIsolationEvent,
) {
    let actual = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("raw isolation event should arrive promptly")
        .expect("raw isolation server should remain available");
    assert_eq!(actual, expected);
}

/// Start an echo server bound to a *specific* address (used to bring a device
/// up after the pool's pre-connect has already started retrying).
async fn echo_server_on(addr: SocketAddr) {
    let config = TcpServerConfig::default();
    let listener = TcpServerListener::bind(addr, config).await.unwrap();

    tokio::spawn(async move {
        while let Ok((mut sink, mut stream, _, _guard)) = listener.accept().await {
            tokio::spawn(async move {
                while let Ok(frame) = stream.recv().await {
                    if sink.send(frame).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
}

fn make_frame(txn: u16, unit: u8, pdu: &[u8]) -> Frame {
    Frame {
        header: FrameHeader::Mbap(MbapHeader::new(txn, unit, pdu.len() as u16)),
        pdu: Bytes::copy_from_slice(pdu),
    }
}

fn pool_config_for(addr: SocketAddr) -> PoolConfig {
    PoolConfig {
        max_connections: 4,
        pre_connect: false,
        tcp_config: TcpConfig {
            port: addr.port(),
            ..TcpConfig::default()
        },
        health_check_interval: Duration::from_secs(300), // don't interfere with tests
        ..PoolConfig::default()
    }
}

fn replenishing_priority_config(addr: SocketAddr, max_connections: usize) -> PoolConfig {
    PoolConfig {
        priority_replenishment: true,
        priority_devices: vec![PriorityDevice::new(addr, max_connections)],
        ..pool_config_for(addr)
    }
}

#[tokio::test]
async fn public_metrics_snapshot_is_root_exported_and_fresh_pool_is_zero() {
    let pool = ConnectionPool::new(PoolConfig {
        pre_connect: false,
        ..PoolConfig::default()
    });
    let snapshot: PoolMetricsSnapshot = pool.metrics();

    assert_eq!(snapshot, PoolMetricsSnapshot::default());
    assert_eq!(snapshot.active_connections, pool.active_count());
    assert_eq!(snapshot.idle_connections, pool.idle_count());
}

#[tokio::test]
async fn pristine_raw_drop_retires_and_next_get_connects_fresh() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));

    let first = pool.get(addr).await.unwrap();
    wait_for_accept(&mut accepted).await;
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(first);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let second = pool.get(addr).await.unwrap();
    wait_for_accept(&mut accepted).await;
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    drop(second);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn successful_raw_send_receive_drop_still_forces_fresh_connection() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));

    {
        let mut conn = pool.get(addr).await.unwrap();
        wait_for_accept(&mut accepted).await;
        let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
        conn.sink().send(make_frame(1, 0xFF, &pdu)).await.unwrap();
        let resp = conn.stream().recv().await.unwrap();
        assert_eq!(&resp.pdu[..], &pdu);
    }
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let second = pool.get(addr).await.unwrap();
    wait_for_accept(&mut accepted).await;
    drop(second);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn raw_cancellation_after_send_keeps_late_response_from_crossing_borrowers() {
    let (addr, mut events, release_late, server) = raw_late_response_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));
    let request_a = [0x03, 0x00, 0x00, 0x00, 0x01];
    let request_b = [0x03, 0x00, 0x01, 0x00, 0x01];

    let mut first = pool.get(addr).await.unwrap();
    expect_raw_event(&mut events, RawIsolationEvent::Accepted(1)).await;
    let first_operation = tokio::spawn(async move {
        first
            .sink()
            .send(make_frame(1, 0xFF, &request_a))
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });
    expect_raw_event(&mut events, RawIsolationEvent::Request(1)).await;
    first_operation.abort();
    assert!(first_operation.await.unwrap_err().is_cancelled());
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let mut second = pool.get(addr).await.unwrap();
    expect_raw_event(&mut events, RawIsolationEvent::Accepted(2)).await;
    second
        .sink()
        .send(make_frame(1, 0xFF, &request_b))
        .await
        .unwrap();
    expect_raw_event(&mut events, RawIsolationEvent::Request(2)).await;
    release_late.send(()).unwrap();
    expect_raw_event(&mut events, RawIsolationEvent::LateResponseAttempted).await;

    let response = second.stream().recv().await.unwrap();
    assert_eq!(&response.pdu[..], &request_b);
    drop(second);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
    server.await.unwrap();
}

#[tokio::test]
async fn unwind_after_either_raw_half_access_releases_once_without_idle() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let mut config = pool_config_for(addr);
    config.max_connections = 2;
    let pool = ConnectionPool::new(config);

    for access_sink in [true, false] {
        let sibling = pool.get(addr).await.unwrap();
        wait_for_accept(&mut accepted).await;
        let lease = pool.get(addr).await.unwrap();
        wait_for_accept(&mut accepted).await;
        assert_eq!(pool.active_count(), 2);
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let mut lease = lease;
            if access_sink {
                let _ = lease.sink();
            } else {
                let _ = lease.stream();
            }
            panic!("test unwind after raw access");
        }));

        assert!(unwind.is_err());
        assert_eq!(
            pool.active_count(),
            1,
            "the sibling charge proves unwind released exactly once"
        );
        drop(sibling);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.idle_count(), 0);
    }
}

#[tokio::test]
async fn invalidating_non_priority_releases_capacity_without_returning_idle() {
    let addr = echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));
    let mut conn = pool.get(addr).await.unwrap();

    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    assert_eq!(conn.invalidation_reason(), None);
    assert!(format!("{conn:?}").contains("invalidation_reason: None"));

    conn.invalidate(LeaseInvalidationReason::CallerDirected);

    assert_eq!(
        conn.invalidation_reason(),
        Some(LeaseInvalidationReason::CallerDirected)
    );
    assert!(format!("{conn:?}").contains("invalidation_reason: Some(CallerDirected)"));
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    drop(conn);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn transport_error_suggestions_do_not_mutate_a_checked_out_lease() {
    let addr = echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));
    let mut conn = pool.get(addr).await.unwrap();

    let suggested =
        LeaseInvalidationReason::suggested_for_transport_error(&TransportError::Disconnected);
    let no_suggestion =
        LeaseInvalidationReason::suggested_for_transport_error(&TransportError::AccessDenied);

    assert_eq!(suggested, Some(LeaseInvalidationReason::Transport));
    assert_eq!(no_suggestion, None);
    assert_eq!(conn.invalidation_reason(), None);
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    assert_eq!(conn.addr(), addr);

    let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
    conn.sink().send(make_frame(1, 0xFF, &pdu)).await.unwrap();
    let response = conn.stream().recv().await.unwrap();
    assert_eq!(&response.pdu[..], &pdu);
    assert_eq!(conn.invalidation_reason(), None);
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    conn.invalidate(suggested.expect("transport error should suggest invalidation"));

    assert_eq!(
        conn.invalidation_reason(),
        Some(LeaseInvalidationReason::Transport)
    );
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn invalidating_priority_releases_its_device_budget() {
    let addr = echo_server().await;
    let mut config = pool_config_for(addr);
    config.priority_devices = vec![PriorityDevice::new(addr, 1)];
    let pool = ConnectionPool::new(config);
    let mut first = pool.get(addr).await.unwrap();

    assert_eq!(pool.active_count(), 1);
    assert!(matches!(pool.get(addr).await, Err(PoolError::Exhausted)));

    first.invalidate(LeaseInvalidationReason::Cancelled);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let second = pool
        .get(addr)
        .await
        .expect("invalidation should release the one-slot priority budget");
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(first);
    assert_eq!(pool.active_count(), 1);
    drop(second);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn repeated_invalidation_keeps_first_reason_and_releases_only_once() {
    let addr = echo_server().await;
    let mut config = pool_config_for(addr);
    config.max_connections = 2;
    let pool = ConnectionPool::new(config);
    let mut invalidated = pool.get(addr).await.unwrap();
    let sibling = pool.get(addr).await.unwrap();
    assert_eq!(pool.active_count(), 2);

    invalidated.invalidate(LeaseInvalidationReason::Timeout);
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    invalidated.invalidate(LeaseInvalidationReason::Protocol);
    invalidated.invalidate(LeaseInvalidationReason::Transport);
    assert_eq!(
        invalidated.invalidation_reason(),
        Some(LeaseInvalidationReason::Timeout)
    );
    // The sibling's remaining charge proves repetition did not reach the
    // saturating accounting path and mask an extra release.
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(invalidated);
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(sibling);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn shutdown_then_invalidation_retires_active_connection() {
    let addr = echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));
    let mut conn = pool.get(addr).await.unwrap();
    assert_eq!(pool.active_count(), 1);

    pool.shutdown();
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    conn.invalidate(LeaseInvalidationReason::Transport);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    drop(conn);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
#[should_panic(expected = "connection already returned or invalidated")]
async fn invalidated_connection_access_panics() {
    let addr = echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));
    let mut conn = pool.get(addr).await.unwrap();
    conn.invalidate(LeaseInvalidationReason::Protocol);

    let _ = conn.addr();
}

#[tokio::test]
async fn invalidation_forces_next_acquisition_to_open_fresh_connection() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let mut config = pool_config_for(addr);
    config.max_connections = 1;
    let pool = ConnectionPool::new(config);

    let mut first = pool.get(addr).await.unwrap();
    wait_for_accept(&mut accepted).await;
    first.invalidate(LeaseInvalidationReason::Transport);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let second = pool.get(addr).await.unwrap();
    wait_for_accept(&mut accepted).await;
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(first);
    assert_eq!(pool.active_count(), 1);
    drop(second);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn exhaustion_when_full() {
    let addr = echo_server().await;
    let mut config = pool_config_for(addr);
    config.max_connections = 2;
    let pool = ConnectionPool::new(config);

    // Check out 2 connections (max).
    let _c1 = pool.get(addr).await.unwrap();
    let _c2 = pool.get(addr).await.unwrap();
    assert_eq!(pool.active_count(), 2);

    // Third should fail.
    let result = pool.get(addr).await;
    assert!(matches!(result, Err(PoolError::Exhausted)));
}

#[tokio::test]
async fn separate_priority_and_non_priority_capacity_budgets_are_publicly_bounded() {
    let (priority_addr, mut priority_accepted) = tracked_echo_server().await;
    let (ordinary_addr, mut ordinary_accepted) = tracked_echo_server().await;
    let mut config = pool_config_for(ordinary_addr);
    config.max_connections = 1;
    config.pre_connect = false;
    config.priority_devices = vec![PriorityDevice::new(priority_addr, 1)];
    let pool = ConnectionPool::new(config);

    let priority = pool.get(priority_addr).await.unwrap();
    wait_for_accept(&mut priority_accepted).await;
    let ordinary = pool.get(ordinary_addr).await.unwrap();
    wait_for_accept(&mut ordinary_accepted).await;
    let metrics = pool.metrics();
    assert_eq!(
        (
            metrics.active_connections,
            metrics.idle_connections,
            metrics.connections_created,
            metrics.connection_failures,
            metrics.connections_retired,
        ),
        (2, 0, 2, 0, 0)
    );

    assert!(matches!(
        pool.get(priority_addr).await,
        Err(PoolError::Exhausted)
    ));
    assert!(
        priority_accepted.try_recv().is_err(),
        "full priority budget must not open another TCP connection"
    );
    assert!(matches!(
        pool.get(ordinary_addr).await,
        Err(PoolError::Exhausted)
    ));
    assert!(
        ordinary_accepted.try_recv().is_err(),
        "full non-priority budget must not open another TCP connection"
    );
    let metrics = pool.metrics();
    assert_eq!(
        (
            metrics.active_connections,
            metrics.idle_connections,
            metrics.connections_created,
            metrics.connection_failures,
            metrics.connections_retired,
        ),
        (2, 0, 2, 0, 0)
    );

    drop(priority);
    drop(ordinary);
    let metrics = pool.metrics();
    assert_eq!(
        (
            metrics.active_connections,
            metrics.idle_connections,
            metrics.connections_created,
            metrics.connection_failures,
            metrics.connections_retired,
        ),
        (0, 0, 2, 0, 2)
    );
    pool.shutdown_and_wait().await;
}

#[tokio::test]
async fn zero_acquisition_timeout_checks_immediate_capacity_after_raw_retirement() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let mut config = pool_config_for(addr);
    config.max_connections = 1;
    let pool = ConnectionPool::new(config);

    let first = pool
        .get_with_acquisition_timeout(addr, Duration::ZERO)
        .await
        .expect("zero timeout should allow an immediate reservation");
    wait_for_accept(&mut accepted).await;

    let result = pool
        .get_with_acquisition_timeout(addr, Duration::ZERO)
        .await;
    assert!(matches!(result, Err(PoolError::Timeout)));

    drop(first);
    let fresh = pool
        .get_with_acquisition_timeout(addr, Duration::ZERO)
        .await
        .expect("zero timeout should allow an immediate fresh reservation");
    wait_for_accept(&mut accepted).await;
    assert_eq!(fresh.addr(), addr);
}

#[tokio::test]
async fn duration_max_allows_public_immediate_capacity_after_raw_retirement() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let mut config = pool_config_for(addr);
    config.max_connections = 1;
    let pool = ConnectionPool::new(config);

    let first = pool
        .get_with_acquisition_timeout(addr, Duration::MAX)
        .await
        .expect("Duration::MAX must not panic before an available reservation");
    wait_for_accept(&mut accepted).await;
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(first);
    let fresh = pool
        .get_with_acquisition_timeout(addr, Duration::MAX)
        .await
        .expect("Duration::MAX must not panic before an immediate fresh reservation");
    wait_for_accept(&mut accepted).await;
    assert_eq!(fresh.addr(), addr);
}

#[tokio::test]
async fn duration_max_full_public_api_returns_timeout_without_accounting_change() {
    let addr = echo_server().await;
    let mut config = pool_config_for(addr);
    config.max_connections = 1;
    let pool = ConnectionPool::new(config);
    let first = pool.get(addr).await.unwrap();
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    let result = pool.get_with_acquisition_timeout(addr, Duration::MAX).await;

    assert!(matches!(result, Err(PoolError::Timeout)));
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(first);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[cfg(feature = "client")]
#[tokio::test]
async fn evicts_oldest_idle_non_priority() {
    let addr1 = echo_server().await;
    let addr2 = echo_server().await;

    let mut config = pool_config_for(addr1);
    config.max_connections = 2;
    let pool = ConnectionPool::new(config);

    // Verdict-gated client returns create two reusable non-priority entries.
    let first = pool
        .get(addr1)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    let second = pool
        .get(addr1)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    assert_eq!(
        first.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    assert_eq!(
        second.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    assert_eq!(pool.idle_count(), 2);

    // Request to addr2 — should evict one idle for addr1.
    let conn = pool.get(addr2).await.unwrap();
    assert_eq!(conn.addr(), addr2);
    assert_eq!(pool.idle_count(), 1); // one addr1 idle remains
    assert_eq!(pool.active_count(), 1);
}

#[tokio::test]
async fn shutdown_rejects_new_requests() {
    let addr = echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));

    pool.shutdown();

    let result = pool.get(addr).await;
    assert!(matches!(result, Err(PoolError::ShuttingDown)));
}

#[cfg(feature = "client")]
#[tokio::test]
async fn priority_connections_not_evicted() {
    let priority_addr = echo_server().await;
    let other_addr = echo_server().await;

    // Non-priority budget of 1; priority device has its own separate budget.
    let mut config = pool_config_for(priority_addr);
    config.max_connections = 1;
    config.priority_devices = vec![PriorityDevice::new(priority_addr, 1)];
    let pool = ConnectionPool::new(config);

    // Verdict-gated client returns create one idle entry in each pool.
    let priority = pool
        .get(priority_addr)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    let ordinary = pool
        .get(other_addr)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    assert_eq!(
        priority.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    assert_eq!(
        ordinary.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    assert_eq!(pool.idle_count(), 2); // 1 priority + 1 non-priority

    // A new non-priority request fills the non-priority pool by evicting the
    // non-priority idle — never the priority one.
    let third_addr = echo_server().await;
    let conn = pool.get(third_addr).await.unwrap();
    assert_eq!(conn.addr(), third_addr);

    // Exactly one idle remains, and it must be the priority connection: the
    // non-priority `other_addr` was evicted, `third_addr` is now active.
    assert_eq!(pool.idle_count(), 1);
    assert_eq!(pool.active_count(), 1);
}

#[tokio::test]
async fn priority_idle_does_not_starve_non_priority() {
    // Regression: idle priority connections used to count toward the single
    // global budget and, being non-evictable, would make every non-priority
    // request fail with `Exhausted`. Separate per-pool budgets must prevent that.
    let p1 = echo_server().await;
    let p2 = echo_server().await;

    let mut config = pool_config_for(p1);
    config.max_connections = 2; // non-priority budget
    config.priority_devices = vec![PriorityDevice::new(p1, 1), PriorityDevice::new(p2, 1)];
    config.pre_connect = true;
    let pool = ConnectionPool::new(config);

    // Pre-connect creates the two idle (never-evicted) priority connections.
    wait_for_idle_count(&pool, 2).await;
    assert_eq!(pool.idle_count(), 2);

    // Two non-priority requests must still succeed despite the full priority pool.
    let np1 = echo_server().await;
    let np2 = echo_server().await;
    let c1 = pool.get(np1).await.unwrap();
    assert_eq!(c1.addr(), np1);
    let c2 = pool.get(np2).await.unwrap();
    assert_eq!(c2.addr(), np2);
    assert_eq!(pool.active_count(), 2);
}

#[tokio::test]
async fn priority_device_respects_per_device_cap() {
    // `PriorityDevice::max_connections` caps a single device even when the global
    // pool has ample room.
    let p = echo_server().await;
    let mut config = pool_config_for(p);
    config.max_connections = 64;
    config.priority_devices = vec![PriorityDevice::new(p, 2)];
    let pool = ConnectionPool::new(config);

    let _c1 = pool.get(p).await.unwrap();
    let _c2 = pool.get(p).await.unwrap();
    assert_eq!(pool.active_count(), 2);

    // Third connection to the same priority device exceeds its per-device cap.
    let result = pool.get(p).await;
    assert!(matches!(result, Err(PoolError::Exhausted)));
}

#[tokio::test]
async fn pre_connect_retries_until_device_up() {
    // Proves the pre-connect loop *retries*: the first attempt fails (device
    // down) and the connection is only established once the device comes up —
    // a single-shot pre-connect would give up and never reconnect. The exact
    // exponential cadence is covered by the backoff unit tests in `backoff.rs`.
    //
    // Reserve an ephemeral port, then free it so the device is initially down.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);

    let mut config = pool_config_for(addr);
    config.pre_connect = true;
    config.backoff = BackoffConfig {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(50),
        multiplier: 2.0,
    };
    config.priority_devices = vec![PriorityDevice::new(addr, 1)];
    let pool = ConnectionPool::new(config);

    // Pre-connect is now failing (connection refused) and retrying with backoff.
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(pool.idle_count(), 0, "device down: no idle connection yet");

    // Bring the device up; a subsequent retry must establish the connection.
    echo_server_on(addr).await;

    let mut established = false;
    for _ in 0..40 {
        if pool.idle_count() == 1 {
            established = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        established,
        "pre-connect backoff retry should establish the priority connection once the device is up"
    );
}

#[tokio::test]
async fn passively_validated_preconnected_entry_can_checkout_but_raw_drop_retires_it() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let mut config = pool_config_for(addr);
    config.pre_connect = true;
    config.priority_devices = vec![PriorityDevice::new(addr, 1)];
    let pool = ConnectionPool::new(config);

    wait_for_accept(&mut accepted).await;
    wait_for_idle_count(&pool, 1).await;
    let checked_out = pool.get(addr).await.unwrap();
    assert!(
        accepted.try_recv().is_err(),
        "checkout should use the passively validated pre-connected entry"
    );
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(checked_out);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let fresh = pool.get(addr).await.unwrap();
    wait_for_accept(&mut accepted).await;
    assert_eq!(fresh.addr(), addr);
    drop(fresh);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn standing_priority_replenishment_restores_idle_after_raw_drop() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let pool = ConnectionPool::new(replenishing_priority_config(addr, 1));

    wait_for_accept(&mut accepted).await;
    wait_for_idle_count(&pool, 1).await;
    let raw = pool
        .get(addr)
        .await
        .expect("standing warm-up entry should be reusable");
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    assert!(accepted.try_recv().is_err());

    drop(raw);
    wait_for_accept(&mut accepted).await;
    wait_for_idle_count(&pool, 1).await;
    assert_eq!(pool.active_count(), 0);

    pool.shutdown_and_wait().await;
}

#[cfg(feature = "client")]
#[tokio::test]
async fn reusable_retirement_replenishes_but_returned_idle_does_not_connect() {
    let (addr, mut accepted) = tracked_echo_server().await;
    let pool = ConnectionPool::new(replenishing_priority_config(addr, 1));

    wait_for_accept(&mut accepted).await;
    wait_for_idle_count(&pool, 1).await;
    let retired = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    retired.client().abort();
    assert!(matches!(
        retired.shutdown_and_return().await,
        PooledClientReturnOutcome::Retired(_)
    ));
    wait_for_accept(&mut accepted).await;
    wait_for_idle_count(&pool, 1).await;

    let returned = pool
        .get(addr)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    assert_eq!(
        returned.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    wait_for_idle_count(&pool, 1).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), accepted.recv())
            .await
            .is_err(),
        "returning the target idle entry must not open another connection"
    );

    pool.shutdown_and_wait().await;
}

/// A refused address: bind an ephemeral port to learn it, then free it.
async fn dead_addr() -> SocketAddr {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap();
    drop(probe);
    addr
}

#[tokio::test]
async fn connect_failure_releases_non_priority_slot() {
    let dead = dead_addr().await;
    let pool = ConnectionPool::new(pool_config_for(dead));

    let result = pool.get(dead).await;
    assert!(matches!(result, Err(PoolError::ConnectionFailed(_))));

    // The reserved slot must be released, not leaked.
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

#[tokio::test]
async fn connect_failure_releases_priority_budget() {
    let dead = dead_addr().await;
    let mut config = pool_config_for(dead);
    config.pre_connect = false;
    config.priority_devices = vec![PriorityDevice::new(dead, 1)];
    let pool = ConnectionPool::new(config);

    let result = pool.get(dead).await;
    assert!(matches!(result, Err(PoolError::ConnectionFailed(_))));
    assert_eq!(pool.active_count(), 0);

    // Budget released — a second attempt is allowed (ConnectionFailed, not
    // Exhausted, proves the per-device count was not leaked).
    let result2 = pool.get(dead).await;
    assert!(matches!(result2, Err(PoolError::ConnectionFailed(_))));
}

#[cfg(feature = "client")]
#[tokio::test]
async fn failed_connect_preserves_evicted_idle() {
    let live = echo_server().await;
    let dead = dead_addr().await;

    let mut config = pool_config_for(live);
    config.max_connections = 1; // non-priority budget of 1
    let pool = ConnectionPool::new(config);

    // A verdict-gated client return creates one idle non-priority connection.
    let session = pool
        .get(live)
        .await
        .unwrap()
        .into_reusable_client(ClientConfig::default())
        .unwrap();
    assert_eq!(
        session.shutdown_and_return().await,
        PooledClientReturnOutcome::ReturnedToIdle
    );
    assert_eq!(pool.idle_count(), 1);

    // get(dead) requires eviction (pool full), but the connect fails. The
    // healthy idle connection must be restored, not destroyed.
    let result = pool.get(dead).await;
    assert!(matches!(result, Err(PoolError::ConnectionFailed(_))));
    assert_eq!(
        pool.idle_count(),
        1,
        "a transient connect failure must not destroy the evicted idle connection"
    );
    assert_eq!(pool.active_count(), 0);
}

#[tokio::test]
async fn dropped_pool_aborts_pre_connect_promptly() {
    let dead = dead_addr().await;

    let metrics = tokio::runtime::Handle::current().metrics();
    let baseline = metrics.num_alive_tasks();

    let mut config = pool_config_for(dead);
    config.pre_connect = true;
    // Long backoff: an un-aborted task would stay parked in its sleep for
    // seconds. Abort must kill it well before that.
    config.backoff = BackoffConfig {
        initial_delay: Duration::from_secs(10),
        max_delay: Duration::from_secs(10),
        multiplier: 1.0,
    };
    config.priority_devices = vec![PriorityDevice::new(dead, 1)];
    let pool = ConnectionPool::new(config);

    // Let the first connect fail and the task park in its 10s backoff sleep.
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(
        metrics.num_alive_tasks() > baseline,
        "pre-connect task should be alive while retrying"
    );

    drop(pool); // Drop -> shutdown() -> abort the pre-connect (and health) tasks.

    // Without the abort the task would remain parked ~10s; with it, it dies fast.
    let mut drained = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        if metrics.num_alive_tasks() <= baseline {
            drained = true;
            break;
        }
    }
    assert!(
        drained,
        "dropping the pool must abort the parked pre-connect task promptly"
    );
}

#[tokio::test]
async fn two_connections_work_independently() {
    let addr = echo_server().await;
    let mut config = pool_config_for(addr);
    config.max_connections = 4;
    let pool = ConnectionPool::new(config);

    let mut c1 = pool.get(addr).await.unwrap();
    let mut c2 = pool.get(addr).await.unwrap();

    // Use both independently.
    let pdu1 = [0x03, 0x00, 0x00, 0x00, 0x01];
    let pdu2 = [0x03, 0x00, 0x01, 0x00, 0x02];

    c1.sink().send(make_frame(1, 0xFF, &pdu1)).await.unwrap();
    c2.sink().send(make_frame(2, 0x01, &pdu2)).await.unwrap();

    let r1 = c1.stream().recv().await.unwrap();
    let r2 = c2.stream().recv().await.unwrap();

    assert_eq!(&r1.pdu[..], &pdu1);
    assert_eq!(&r2.pdu[..], &pdu2);
}
