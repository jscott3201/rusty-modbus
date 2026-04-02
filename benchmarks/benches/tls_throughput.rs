//! TLS transport throughput benchmarks — batched operations.

#![allow(clippy::await_holding_refcell_ref)]

use std::cell::RefCell;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use rusty_modbus_benchmarks::frame_builders::*;
use rusty_modbus_benchmarks::helpers::make_store;
use rusty_modbus_benchmarks::tls_helpers::{generate_test_certs, make_tls_client, make_tls_server};
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use tokio::runtime::Runtime;

fn bench_tls_read_holding_registers_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let certs = generate_test_certs();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_tls_server(&certs, store));
    let (sink, stream) = rt.block_on(make_tls_client(&certs, addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let txn_id = RefCell::new(0u16);

    let mut group = c.benchmark_group("tls_read_holding_registers_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let sink = &sink;
                let stream = &stream;
                let txn_id = &txn_id;
                b.to_async(&rt).iter(|| async move {
                    for _ in 0..n {
                        let frame = {
                            let mut id = txn_id.borrow_mut();
                            *id = id.wrapping_add(1);
                            read_holding_registers_mbap(*id, 1, 0, 10)
                        };
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

fn bench_tls_write_single_register_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let certs = generate_test_certs();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_tls_server(&certs, store));
    let (sink, stream) = rt.block_on(make_tls_client(&certs, addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let txn_id = RefCell::new(0u16);

    let mut group = c.benchmark_group("tls_write_single_register_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let sink = &sink;
                let stream = &stream;
                let txn_id = &txn_id;
                b.to_async(&rt).iter(|| async move {
                    for i in 0..n {
                        let frame = {
                            let mut id = txn_id.borrow_mut();
                            *id = id.wrapping_add(1);
                            write_single_register_mbap(*id, 1, (i % 100) as u16, 0x1234)
                        };
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

fn bench_tls_mixed_read_write_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let certs = generate_test_certs();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_tls_server(&certs, store));
    let (sink, stream) = rt.block_on(make_tls_client(&certs, addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let txn_id = RefCell::new(0u16);

    let mut group = c.benchmark_group("tls_mixed_read_write_throughput");
    for batch_size in [10u64, 100, 1000] {
        group.throughput(Throughput::Elements(batch_size));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &n| {
                let sink = &sink;
                let stream = &stream;
                let txn_id = &txn_id;
                b.to_async(&rt).iter(|| async move {
                    for i in 0..n {
                        let cur_id = {
                            let mut id = txn_id.borrow_mut();
                            *id = id.wrapping_add(1);
                            *id
                        };
                        if i % 2 == 0 {
                            let frame = read_holding_registers_mbap(cur_id, 1, 0, 10);
                            sink.borrow_mut().send(frame).await.unwrap();
                        } else {
                            let frame = write_single_register_mbap(cur_id, 1, 0, 0x1234);
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
    bench_tls_read_holding_registers_throughput,
    bench_tls_write_single_register_throughput,
    bench_tls_mixed_read_write_throughput,
);
criterion_main!(benches);
