//! Observational benchmarks for uncontended public connection-pool lifecycle paths.

use std::time::{Duration, Instant};

use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, Criterion, black_box, criterion_group, criterion_main};
use rusty_modbus_benchmarks::helpers::make_tcp_server;
use rusty_modbus_pool::{
    ClientConfig, ConnectionPool, PoolConfig, PoolMetricsSnapshot, PooledClientReturnOutcome,
    RetryConfig,
};
use rusty_modbus_server::ShutdownOutcome;
use rusty_modbus_types::UnitId;
use tokio::runtime::Runtime;

const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(1);

fn pool_config() -> PoolConfig {
    PoolConfig {
        max_connections: 1,
        priority_devices: Vec::new(),
        pre_connect: false,
        priority_replenishment: false,
        idle_timeout: MAINTENANCE_INTERVAL,
        health_check_interval: MAINTENANCE_INTERVAL,
        ..PoolConfig::default()
    }
}

fn client_config() -> ClientConfig {
    ClientConfig {
        unit_id: UnitId(0xff),
        timeout: CLIENT_TIMEOUT,
        max_in_flight: 1,
        retry: RetryConfig {
            max_retries: 0,
            retry_delay: Duration::ZERO,
            retryable_exceptions: Vec::new(),
        },
        shutdown_timeout: CLIENT_TIMEOUT,
    }
}

fn assert_metrics(
    actual: PoolMetricsSnapshot,
    active: usize,
    idle: usize,
    created: u64,
    failures: u64,
    retired: u64,
) {
    assert_eq!(actual.active_connections, active);
    assert_eq!(actual.idle_connections, idle);
    assert_eq!(actual.connections_created, created);
    assert_eq!(actual.connection_failures, failures);
    assert_eq!(actual.connections_retired, retired);
}

fn bench_fresh_get_raw_drop(group: &mut BenchmarkGroup<'_, WallTime>) {
    let runtime = Runtime::new().expect("benchmark runtime must start");
    let (server, addr, pool) = runtime.block_on(async {
        let (server, addr) = make_tcp_server().await;
        (server, addr, ConnectionPool::new(pool_config()))
    });
    assert_metrics(pool.metrics(), 0, 0, 0, 0, 0);

    let mut total_iterations = 0_u64;
    group.bench_function("fresh_get_raw_drop", |bencher| {
        let pool = &pool;
        bencher.to_async(&runtime).iter_custom(|iterations| {
            total_iterations = total_iterations
                .checked_add(iterations)
                .expect("fresh benchmark iteration count must not overflow");
            async move {
                let before = pool.metrics();
                let start = Instant::now();
                for _ in 0..iterations {
                    let lease = pool
                        .get(addr)
                        .await
                        .expect("fresh pool acquisition must succeed");
                    let _observed_addr = black_box(lease.addr());
                    drop(lease);
                }
                let elapsed = start.elapsed();

                let after = pool.metrics();
                assert_eq!(before.active_connections, 0);
                assert_eq!(before.idle_connections, 0);
                assert_eq!(before.connection_failures, 0);
                assert_eq!(before.connections_created, before.connections_retired);
                assert_eq!(after.active_connections, 0);
                assert_eq!(after.idle_connections, 0);
                assert_eq!(after.connection_failures, before.connection_failures);
                assert_eq!(after.connection_failures, 0);
                assert_eq!(
                    after.connections_created,
                    before
                        .connections_created
                        .checked_add(iterations)
                        .expect("created-connection counter delta must not overflow")
                );
                assert_eq!(
                    after.connections_retired,
                    before
                        .connections_retired
                        .checked_add(iterations)
                        .expect("retired-connection counter delta must not overflow")
                );
                assert_eq!(after.connections_created, after.connections_retired);
                elapsed
            }
        });
    });

    assert!(
        total_iterations > 0,
        "fresh benchmark must run an iteration"
    );
    let before_shutdown = pool.metrics();
    assert_eq!(before_shutdown.active_connections, 0);
    assert_eq!(before_shutdown.idle_connections, 0);
    assert_eq!(before_shutdown.connection_failures, 0);
    assert_eq!(
        before_shutdown.connections_created,
        before_shutdown.connections_retired
    );
    runtime.block_on(async {
        pool.shutdown_and_wait().await;
        assert_metrics(
            pool.metrics(),
            0,
            0,
            before_shutdown.connections_created,
            0,
            before_shutdown.connections_retired,
        );
        assert_eq!(server.stop().await, ShutdownOutcome::Drained);
    });
}

fn bench_reusable_checkout_handoff_shutdown_return(group: &mut BenchmarkGroup<'_, WallTime>) {
    let runtime = Runtime::new().expect("benchmark runtime must start");
    let (server, addr, pool) = runtime.block_on(async {
        let (server, addr) = make_tcp_server().await;
        (server, addr, ConnectionPool::new(pool_config()))
    });

    runtime.block_on(async {
        let seed = pool
            .get(addr)
            .await
            .expect("reusable seed acquisition must succeed")
            .into_reusable_client(client_config())
            .expect("pristine reusable seed handoff must succeed");
        assert_eq!(
            seed.shutdown_and_return().await,
            PooledClientReturnOutcome::ReturnedToIdle
        );
    });
    assert_metrics(pool.metrics(), 0, 1, 1, 0, 0);

    let mut total_iterations = 0_u64;
    group.bench_function("reusable_checkout_handoff_shutdown_return", |bencher| {
        let pool = &pool;
        bencher.to_async(&runtime).iter_custom(|iterations| {
            total_iterations = total_iterations
                .checked_add(iterations)
                .expect("reusable benchmark iteration count must not overflow");
            async move {
                let mut all_returned_to_idle = true;
                let start = Instant::now();
                for _ in 0..iterations {
                    let session = pool
                        .get(addr)
                        .await
                        .expect("reusable pool acquisition must succeed")
                        .into_reusable_client(client_config())
                        .expect("pristine reusable handoff must succeed");
                    let outcome = session.shutdown_and_return().await;
                    all_returned_to_idle &= outcome == PooledClientReturnOutcome::ReturnedToIdle;
                }
                let elapsed = start.elapsed();

                assert!(
                    all_returned_to_idle,
                    "every reusable session must return to idle"
                );
                assert_metrics(pool.metrics(), 0, 1, 1, 0, 0);
                elapsed
            }
        });
    });

    assert!(
        total_iterations > 0,
        "reusable benchmark must run an iteration"
    );
    assert_metrics(pool.metrics(), 0, 1, 1, 0, 0);
    runtime.block_on(async {
        pool.shutdown_and_wait().await;
        assert_metrics(pool.metrics(), 0, 0, 1, 0, 1);
        assert_eq!(server.stop().await, ShutdownOutcome::Drained);
    });
}

fn bench_tcp_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("tcp_pool");
    bench_fresh_get_raw_drop(&mut group);
    bench_reusable_checkout_handoff_shutdown_return(&mut group);
    group.finish();
}

criterion_group!(benches, bench_tcp_pool);
criterion_main!(benches);
