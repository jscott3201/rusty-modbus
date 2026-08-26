//! Integration tests for the connection pool.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_pool::{
    BackoffConfig, ConnectionPool, LeaseInvalidationReason, PoolConfig, PoolError, PriorityDevice,
};
use rusty_modbus_tcp::TransportError;
use rusty_modbus_tcp::config::{TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::MbapHeader;
use tokio::sync::mpsc;

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

#[tokio::test]
async fn get_and_return_cycle() {
    let addr = echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));

    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    // Get a connection.
    {
        let mut conn = pool.get(addr).await.unwrap();
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);

        // Use it.
        let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
        conn.sink().send(make_frame(1, 0xFF, &pdu)).await.unwrap();
        let resp = conn.stream().recv().await.unwrap();
        assert_eq!(&resp.pdu[..], &pdu);
    }
    // Dropped — should return to idle.
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 1);
}

#[tokio::test]
async fn reuses_idle_connection() {
    let addr = echo_server().await;
    let pool = ConnectionPool::new(pool_config_for(addr));

    // Get and return.
    {
        let _conn = pool.get(addr).await.unwrap();
    }
    assert_eq!(pool.idle_count(), 1);

    // Get again — should reuse the idle connection (no new TCP handshake).
    {
        let mut conn = pool.get(addr).await.unwrap();
        assert_eq!(pool.active_count(), 1);
        assert_eq!(pool.idle_count(), 0);

        // Verify it still works.
        let pdu = [0x03, 0x00, 0x00, 0x00, 0x01];
        conn.sink().send(make_frame(1, 0xFF, &pdu)).await.unwrap();
        let resp = conn.stream().recv().await.unwrap();
        assert_eq!(&resp.pdu[..], &pdu);
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
    config.priority_devices = vec![PriorityDevice {
        addr,
        max_connections: 1,
    }];
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
    assert_eq!(pool.idle_count(), 1);
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
    assert_eq!(pool.idle_count(), 1);
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
    assert_eq!(pool.idle_count(), 1);
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
async fn zero_acquisition_timeout_checks_immediate_capacity_and_reuse() {
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
    let reused = pool
        .get_with_acquisition_timeout(addr, Duration::ZERO)
        .await
        .expect("zero timeout should allow immediate idle reuse");
    assert_eq!(reused.addr(), addr);
    assert!(
        accepted.try_recv().is_err(),
        "idle reuse must not establish another TCP connection"
    );
}

#[tokio::test]
async fn evicts_oldest_idle_non_priority() {
    let addr1 = echo_server().await;
    let addr2 = echo_server().await;

    let mut config = pool_config_for(addr1);
    config.max_connections = 2;
    let pool = ConnectionPool::new(config);

    // Fill pool with 2 idle connections to addr1.
    {
        let _c1 = pool.get(addr1).await.unwrap();
        let _c2 = pool.get(addr1).await.unwrap();
    }
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

#[tokio::test]
async fn priority_connections_not_evicted() {
    let priority_addr = echo_server().await;
    let other_addr = echo_server().await;

    // Non-priority budget of 1; priority device has its own separate budget.
    let mut config = pool_config_for(priority_addr);
    config.max_connections = 1;
    config.priority_devices = vec![PriorityDevice {
        addr: priority_addr,
        max_connections: 1,
    }];
    let pool = ConnectionPool::new(config);

    // One idle priority connection (drawn from the priority pool).
    {
        let _p = pool.get(priority_addr).await.unwrap();
    }
    // Fill the non-priority pool (budget = 1) with one idle connection.
    {
        let _a = pool.get(other_addr).await.unwrap();
    }
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
    config.priority_devices = vec![
        PriorityDevice {
            addr: p1,
            max_connections: 1,
        },
        PriorityDevice {
            addr: p2,
            max_connections: 1,
        },
    ];
    let pool = ConnectionPool::new(config);

    // Saturate the priority pool with two idle (never-evicted) connections.
    {
        let _a = pool.get(p1).await.unwrap();
    }
    {
        let _b = pool.get(p2).await.unwrap();
    }
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
    config.priority_devices = vec![PriorityDevice {
        addr: p,
        max_connections: 2,
    }];
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
    config.priority_devices = vec![PriorityDevice {
        addr,
        max_connections: 1,
    }];
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
    config.priority_devices = vec![PriorityDevice {
        addr: dead,
        max_connections: 1,
    }];
    let pool = ConnectionPool::new(config);

    let result = pool.get(dead).await;
    assert!(matches!(result, Err(PoolError::ConnectionFailed(_))));
    assert_eq!(pool.active_count(), 0);

    // Budget released — a second attempt is allowed (ConnectionFailed, not
    // Exhausted, proves the per-device count was not leaked).
    let result2 = pool.get(dead).await;
    assert!(matches!(result2, Err(PoolError::ConnectionFailed(_))));
}

#[tokio::test]
async fn failed_connect_preserves_evicted_idle() {
    let live = echo_server().await;
    let dead = dead_addr().await;

    let mut config = pool_config_for(live);
    config.max_connections = 1; // non-priority budget of 1
    let pool = ConnectionPool::new(config);

    // One idle non-priority connection to `live` fills the pool.
    {
        let _c = pool.get(live).await.unwrap();
    }
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
    config.priority_devices = vec![PriorityDevice {
        addr: dead,
        max_connections: 1,
    }];
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
