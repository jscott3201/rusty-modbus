//! RTU-over-TCP transport throughput benchmarks — batched operations.

use std::cell::RefCell;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rusty_modbus_benchmarks::frame_builders::*;
use rusty_modbus_benchmarks::helpers::make_store;
use rusty_modbus_benchmarks::rtu_helpers::{make_rtu_tcp_client, make_rtu_tcp_server};
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use tokio::runtime::Runtime;

fn bench_rtu_tcp_read_holding_registers_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_rtu_tcp_server(store));
    let (sink, stream) = rt.block_on(make_rtu_tcp_client(addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);

    let mut group = c.benchmark_group("rtu_tcp_read_holding_registers_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let sink = &sink;
                let stream = &stream;
                b.to_async(&rt).iter(|| async move {
                    for _ in 0..n {
                        let frame = read_holding_registers_rtu(1, 0, 10);
                        sink.borrow_mut().send(frame).await.unwrap();
                        stream.borrow_mut().recv().await.unwrap();
                    }
                });
            },
        );
    }
    group.finish();
    server_handle.abort();
}

fn bench_rtu_tcp_write_single_register_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_rtu_tcp_server(store));
    let (sink, stream) = rt.block_on(make_rtu_tcp_client(addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);

    let mut group = c.benchmark_group("rtu_tcp_write_single_register_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let sink = &sink;
                let stream = &stream;
                b.to_async(&rt).iter(|| async move {
                    for i in 0..n {
                        let frame = write_single_register_rtu(1, (i % 100) as u16, 0x1234);
                        sink.borrow_mut().send(frame).await.unwrap();
                        stream.borrow_mut().recv().await.unwrap();
                    }
                });
            },
        );
    }
    group.finish();
    server_handle.abort();
}

fn bench_rtu_tcp_mixed_read_write_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_rtu_tcp_server(store));
    let (sink, stream) = rt.block_on(make_rtu_tcp_client(addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);

    let mut group = c.benchmark_group("rtu_tcp_mixed_read_write_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let sink = &sink;
                let stream = &stream;
                b.to_async(&rt).iter(|| async move {
                    for i in 0..n {
                        if i % 2 == 0 {
                            let frame = read_holding_registers_rtu(1, 0, 10);
                            sink.borrow_mut().send(frame).await.unwrap();
                        } else {
                            let frame = write_single_register_rtu(1, 0, 0x1234);
                            sink.borrow_mut().send(frame).await.unwrap();
                        }
                        stream.borrow_mut().recv().await.unwrap();
                    }
                });
            },
        );
    }
    group.finish();
    server_handle.abort();
}

criterion_group!(
    benches,
    bench_rtu_tcp_read_holding_registers_throughput,
    bench_rtu_tcp_write_single_register_throughput,
    bench_rtu_tcp_mixed_read_write_throughput,
);
criterion_main!(benches);
