//! TCP transport latency benchmarks — single request round-trip.

use criterion::{criterion_group, criterion_main, Criterion};
use rusty_modbus_benchmarks::helpers::{current_rss_bytes, make_tcp_client, make_tcp_server};
use rusty_modbus_types::UnitId;
use tokio::runtime::Runtime;

fn bench_tcp_read_holding_registers_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server, addr) = rt.block_on(make_tcp_server());
    let client = rt.block_on(make_tcp_client(addr));
    let rss_before = current_rss_bytes();

    c.bench_function("tcp_read_holding_registers_latency", |b| {
        b.to_async(&rt).iter(|| async {
            client
                .read_holding_registers(UnitId(1), 0, 10)
                .await
                .unwrap();
        });
    });

    let rss_after = current_rss_bytes();
    eprintln!(
        "RSS: before={}KB after={}KB delta={}KB",
        rss_before / 1024,
        rss_after / 1024,
        (rss_after as i64 - rss_before as i64) / 1024
    );
    rt.block_on(async { server.stop().await });
}

fn bench_tcp_write_single_register_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server, addr) = rt.block_on(make_tcp_server());
    let client = rt.block_on(make_tcp_client(addr));

    c.bench_function("tcp_write_single_register_latency", |b| {
        b.to_async(&rt).iter(|| async {
            client
                .write_single_register(UnitId(1), 0, 0x1234)
                .await
                .unwrap();
        });
    });

    rt.block_on(async { server.stop().await });
}

fn bench_tcp_read_coils_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server, addr) = rt.block_on(make_tcp_server());
    let client = rt.block_on(make_tcp_client(addr));

    c.bench_function("tcp_read_coils_latency", |b| {
        b.to_async(&rt).iter(|| async {
            client.read_coils(UnitId(1), 0, 100).await.unwrap();
        });
    });

    rt.block_on(async { server.stop().await });
}

fn bench_tcp_write_multiple_registers_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server, addr) = rt.block_on(make_tcp_server());
    let client = rt.block_on(make_tcp_client(addr));
    let values: Vec<u16> = (0..10).collect();

    c.bench_function("tcp_write_multiple_registers_latency", |b| {
        let vals = &values;
        b.to_async(&rt).iter(|| async {
            client
                .write_multiple_registers(UnitId(1), 0, vals)
                .await
                .unwrap();
        });
    });

    rt.block_on(async { server.stop().await });
}

criterion_group!(
    benches,
    bench_tcp_read_holding_registers_latency,
    bench_tcp_write_single_register_latency,
    bench_tcp_read_coils_latency,
    bench_tcp_write_multiple_registers_latency,
);
criterion_main!(benches);
