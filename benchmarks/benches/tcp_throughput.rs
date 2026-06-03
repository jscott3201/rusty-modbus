//! TCP transport throughput benchmarks — batched operations.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use futures_util::stream::{FuturesUnordered, StreamExt};
use rusty_modbus_benchmarks::helpers::{make_tcp_client, make_tcp_server};
use rusty_modbus_client::ModbusClient;
use rusty_modbus_types::UnitId;
use tokio::runtime::Runtime;

const PIPELINED_OPS_PER_ITER: u64 = 256;

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

async fn run_pipelined_reads(client: &ModbusClient, total_ops: u64, depth: usize) {
    let mut pending = FuturesUnordered::new();
    let mut submitted = 0u64;
    let mut completed = 0u64;

    while completed < total_ops {
        while submitted < total_ops && pending.len() < depth {
            pending.push(client.read_holding_registers(UnitId(1), 0, 10));
            submitted += 1;
        }

        let response = pending
            .next()
            .await
            .expect("pending future exists while completed < total_ops")
            .unwrap();
        black_box(response);
        completed += 1;
    }
}

fn bench_tcp_pipelined_read_holding_registers_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server, addr) = rt.block_on(make_tcp_server());
    let client = rt.block_on(make_tcp_client(addr));

    let mut group = c.benchmark_group("tcp_pipelined_read_holding_registers_throughput");
    group.throughput(Throughput::Elements(PIPELINED_OPS_PER_ITER));
    for depth in [1usize, 2, 4, 8, 16] {
        group.bench_with_input(BenchmarkId::new("depth", depth), &depth, |b, &depth| {
            b.to_async(&rt)
                .iter(|| run_pipelined_reads(&client, PIPELINED_OPS_PER_ITER, black_box(depth)));
        });
    }
    group.finish();
    rt.block_on(async { server.stop().await });
}

criterion_group!(
    benches,
    bench_tcp_read_holding_registers_throughput,
    bench_tcp_write_single_register_throughput,
    bench_tcp_mixed_read_write_throughput,
    bench_tcp_pipelined_read_holding_registers_throughput,
);
criterion_main!(benches);
