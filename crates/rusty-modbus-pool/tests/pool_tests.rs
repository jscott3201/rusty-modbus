//! Integration tests for the connection pool.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use rusty_modbus_frame::frame::{Frame, FrameHeader};
use rusty_modbus_pool::{ConnectionPool, PoolConfig, PoolError, PriorityDevice};
use rusty_modbus_tcp::config::{TcpConfig, TcpServerConfig};
use rusty_modbus_tcp::listener::TcpServerListener;
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::MbapHeader;

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

    let mut config = pool_config_for(priority_addr);
    config.max_connections = 2;
    config.priority_devices = vec![PriorityDevice {
        addr: priority_addr,
        max_connections: 2,
    }];
    let pool = ConnectionPool::new(config);

    // Fill pool: 1 priority idle + 1 non-priority idle.
    {
        let _c1 = pool.get(priority_addr).await.unwrap();
    }
    {
        let _c2 = pool.get(other_addr).await.unwrap();
    }
    assert_eq!(pool.idle_count(), 2);

    // New request for a third address — should evict non-priority, not priority.
    let third_addr = echo_server().await;
    let conn = pool.get(third_addr).await.unwrap();
    assert_eq!(conn.addr(), third_addr);
    // One idle remaining: the priority connection.
    assert_eq!(pool.idle_count(), 1);
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
