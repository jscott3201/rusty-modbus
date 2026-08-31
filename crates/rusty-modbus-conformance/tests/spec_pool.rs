//! Connection pool conformance tests.
//!
//! Verifies the two-pool model and raw-lease retirement safety floor.

use std::net::SocketAddr;
use std::time::Duration;

use rusty_modbus_pool::{BackoffConfig, ConnectionPool, PoolConfig};
use rusty_modbus_tcp::config::TcpConfig;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

// ── Pool Config Defaults ──────────────────────────────────────────

#[test]
fn spec_4_2_1_pool_defaults() {
    let config = PoolConfig::default();
    // Reasonable defaults per spec guidance
    assert!(config.max_connections > 0);
    assert!(config.idle_timeout > Duration::ZERO);
    assert!(config.health_check_interval > Duration::ZERO);
}

#[test]
fn spec_4_2_1_pre_connect_default() {
    let config = PoolConfig::default();
    // Pre-connect to priority devices is recommended
    assert!(config.pre_connect);
}

// ── Backoff Config ────────────────────────────────────────────────

#[test]
fn backoff_defaults_reasonable() {
    let config = BackoffConfig::default();
    // Initial delay should be small
    assert!(config.initial_delay <= Duration::from_secs(1));
    // Max delay should cap
    assert!(config.max_delay >= Duration::from_secs(1));
    // Multiplier > 1 for exponential growth
    assert!(config.multiplier > 1.0);
}

async fn tracked_tcp_server() -> (SocketAddr, mpsc::UnboundedReceiver<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            if accepted_tx.send(()).is_err() {
                return;
            }
            tokio::spawn(async move {
                let mut buffer = [0_u8; 64];
                while socket.read(&mut buffer).await.unwrap_or(0) != 0 {}
            });
        }
    });

    (addr, accepted_rx)
}

async fn expect_accept(accepted: &mut mpsc::UnboundedReceiver<()>) {
    tokio::time::timeout(Duration::from_secs(1), accepted.recv())
        .await
        .expect("fresh TCP connection should be accepted promptly")
        .expect("tracked TCP server should remain available");
}

#[tokio::test]
async fn raw_pool_lease_drop_retires_and_next_checkout_connects_fresh() {
    let (addr, mut accepted) = tracked_tcp_server().await;
    let pool = ConnectionPool::new(PoolConfig {
        max_connections: 1,
        pre_connect: false,
        health_check_interval: Duration::from_secs(300),
        tcp_config: TcpConfig {
            port: addr.port(),
            ..TcpConfig::default()
        },
        ..PoolConfig::default()
    });

    let first = pool.get(addr).await.unwrap();
    expect_accept(&mut accepted).await;
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);

    drop(first);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);

    let second = pool.get(addr).await.unwrap();
    expect_accept(&mut accepted).await;
    assert_eq!(pool.active_count(), 1);
    assert_eq!(pool.idle_count(), 0);
    drop(second);
    assert_eq!(pool.active_count(), 0);
    assert_eq!(pool.idle_count(), 0);
}

// ── Two-Pool Model (via integration tests in pool crate) ──────────
// The existing pool_tests.rs already covers:
// - Priority connections not evicted ✓
// - Oldest non-priority evicted first ✓
// - Exhaustion when full ✓
// - Verdict-gated idle connection reuse ✓
// - Shutdown rejects new requests ✓
// These are conformance-verified by the existing tests.
