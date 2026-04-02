//! TLS transport latency benchmarks — single request round-trip.

#![allow(clippy::await_holding_refcell_ref)]

use std::cell::RefCell;

use criterion::{Criterion, criterion_group, criterion_main};
use rusty_modbus_benchmarks::frame_builders::*;
use rusty_modbus_benchmarks::helpers::{current_rss_bytes, make_store};
use rusty_modbus_benchmarks::tls_helpers::{generate_test_certs, make_tls_client, make_tls_server};
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use tokio::runtime::Runtime;

fn bench_tls_read_holding_registers_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let certs = generate_test_certs();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_tls_server(&certs, store));
    let (sink, stream) = rt.block_on(make_tls_client(&certs, addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let rss_before = current_rss_bytes();
    let txn_id = RefCell::new(0u16);

    c.bench_function("tls_read_holding_registers_latency", |b| {
        b.to_async(&rt).iter(|| {
            let mut id = txn_id.borrow_mut();
            *id = id.wrapping_add(1);
            let frame = read_holding_registers_mbap(*id, 1, 0, 10);
            let sink = &sink;
            let stream = &stream;
            async move {
                sink.borrow_mut().send(frame).await.unwrap();
                stream.borrow_mut().recv().await.unwrap();
            }
        });
    });

    let rss_after = current_rss_bytes();
    eprintln!(
        "RSS: before={}KB after={}KB delta={}KB",
        rss_before / 1024,
        rss_after / 1024,
        (rss_after as i64 - rss_before as i64) / 1024
    );
    server_handle.abort();
}

fn bench_tls_write_single_register_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let certs = generate_test_certs();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_tls_server(&certs, store));
    let (sink, stream) = rt.block_on(make_tls_client(&certs, addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let txn_id = RefCell::new(0u16);

    c.bench_function("tls_write_single_register_latency", |b| {
        b.to_async(&rt).iter(|| {
            let mut id = txn_id.borrow_mut();
            *id = id.wrapping_add(1);
            let frame = write_single_register_mbap(*id, 1, 0, 0x1234);
            let sink = &sink;
            let stream = &stream;
            async move {
                sink.borrow_mut().send(frame).await.unwrap();
                stream.borrow_mut().recv().await.unwrap();
            }
        });
    });

    server_handle.abort();
}

fn bench_tls_read_coils_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let certs = generate_test_certs();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_tls_server(&certs, store));
    let (sink, stream) = rt.block_on(make_tls_client(&certs, addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let txn_id = RefCell::new(0u16);

    c.bench_function("tls_read_coils_latency", |b| {
        b.to_async(&rt).iter(|| {
            let mut id = txn_id.borrow_mut();
            *id = id.wrapping_add(1);
            let frame = read_coils_mbap(*id, 1, 0, 100);
            let sink = &sink;
            let stream = &stream;
            async move {
                sink.borrow_mut().send(frame).await.unwrap();
                stream.borrow_mut().recv().await.unwrap();
            }
        });
    });

    server_handle.abort();
}

fn bench_tls_write_multiple_registers_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let certs = generate_test_certs();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_tls_server(&certs, store));
    let (sink, stream) = rt.block_on(make_tls_client(&certs, addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let txn_id = RefCell::new(0u16);
    let values: Vec<u16> = (0..10).collect();

    c.bench_function("tls_write_multiple_registers_latency", |b| {
        b.to_async(&rt).iter(|| {
            let mut id = txn_id.borrow_mut();
            *id = id.wrapping_add(1);
            let frame = write_multiple_registers_mbap(*id, 1, 0, &values);
            let sink = &sink;
            let stream = &stream;
            async move {
                sink.borrow_mut().send(frame).await.unwrap();
                stream.borrow_mut().recv().await.unwrap();
            }
        });
    });

    server_handle.abort();
}

criterion_group!(
    benches,
    bench_tls_read_holding_registers_latency,
    bench_tls_write_single_register_latency,
    bench_tls_read_coils_latency,
    bench_tls_write_multiple_registers_latency,
);
criterion_main!(benches);
