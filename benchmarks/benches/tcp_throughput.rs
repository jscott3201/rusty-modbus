//! TCP transport throughput benchmarks — batched operations.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use modbus_benchmarks::helpers::{make_tcp_client, make_tcp_server};
use modbus_types::UnitId;
use tokio::runtime::Runtime;

fn bench_tcp_read_holding_registers_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server, addr) = rt.block_on(make_tcp_server());
    let client = rt.block_on(make_tcp_client(addr));

    let mut group = c.benchmark_group("tcp_read_holding_registers_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                b.to_async(&rt).iter(|| async {
                    for _ in 0..n {
                        client
                            .read_holding_registers(UnitId(1), 0, 10)
                            .await
                            .unwrap();
                    }
                });
            },
        );
    }
    group.finish();
    rt.block_on(async { server.stop().await });
}

fn bench_tcp_write_single_register_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server, addr) = rt.block_on(make_tcp_server());
    let client = rt.block_on(make_tcp_client(addr));

    let mut group = c.benchmark_group("tcp_write_single_register_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                b.to_async(&rt).iter(|| async {
                    for i in 0..n {
                        client
                            .write_single_register(UnitId(1), (i % 100) as u16, 0x1234)
                            .await
                            .unwrap();
                    }
                });
            },
        );
    }
    group.finish();
    rt.block_on(async { server.stop().await });
}

fn bench_tcp_mixed_read_write_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server, addr) = rt.block_on(make_tcp_server());
    let client = rt.block_on(make_tcp_client(addr));

    let mut group = c.benchmark_group("tcp_mixed_read_write_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                b.to_async(&rt).iter(|| async {
                    for i in 0..n {
                        if i % 2 == 0 {
                            client
                                .read_holding_registers(UnitId(1), 0, 10)
                                .await
                                .unwrap();
                        } else {
                            client
                                .write_single_register(UnitId(1), 0, 0x1234)
                                .await
                                .unwrap();
                        }
                    }
                });
            },
        );
    }
    group.finish();
    rt.block_on(async { server.stop().await });
}

criterion_group!(
    benches,
    bench_tcp_read_holding_registers_throughput,
    bench_tcp_write_single_register_throughput,
    bench_tcp_mixed_read_write_throughput,
);
criterion_main!(benches);
