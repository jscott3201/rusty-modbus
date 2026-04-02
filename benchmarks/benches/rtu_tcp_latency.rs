//! RTU-over-TCP transport latency benchmarks — single request round-trip.

#![allow(clippy::await_holding_refcell_ref)]

use std::cell::RefCell;

use criterion::{Criterion, criterion_group, criterion_main};
use rusty_modbus_benchmarks::frame_builders::*;
use rusty_modbus_benchmarks::helpers::{current_rss_bytes, make_store};
use rusty_modbus_benchmarks::rtu_helpers::{make_rtu_tcp_client, make_rtu_tcp_server};
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use tokio::runtime::Runtime;

fn bench_rtu_tcp_read_holding_registers_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_rtu_tcp_server(store));
    let (sink, stream) = rt.block_on(make_rtu_tcp_client(addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let rss_before = current_rss_bytes();

    c.bench_function("rtu_tcp_read_holding_registers_latency", |b| {
        let sink = &sink;
        let stream = &stream;
        b.to_async(&rt).iter(|| async move {
            let frame = read_holding_registers_rtu(1, 0, 10);
            sink.borrow_mut().send(frame).await.unwrap();
            stream.borrow_mut().recv().await.unwrap();
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

fn bench_rtu_tcp_write_single_register_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_rtu_tcp_server(store));
    let (sink, stream) = rt.block_on(make_rtu_tcp_client(addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);

    c.bench_function("rtu_tcp_write_single_register_latency", |b| {
        let sink = &sink;
        let stream = &stream;
        b.to_async(&rt).iter(|| async move {
            let frame = write_single_register_rtu(1, 0, 0x1234);
            sink.borrow_mut().send(frame).await.unwrap();
            stream.borrow_mut().recv().await.unwrap();
        });
    });

    server_handle.abort();
}

fn bench_rtu_tcp_read_coils_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_rtu_tcp_server(store));
    let (sink, stream) = rt.block_on(make_rtu_tcp_client(addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);

    c.bench_function("rtu_tcp_read_coils_latency", |b| {
        let sink = &sink;
        let stream = &stream;
        b.to_async(&rt).iter(|| async move {
            let frame = read_coils_rtu(1, 0, 100);
            sink.borrow_mut().send(frame).await.unwrap();
            stream.borrow_mut().recv().await.unwrap();
        });
    });

    server_handle.abort();
}

fn bench_rtu_tcp_write_multiple_registers_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = make_store();
    let (server_handle, addr) = rt.block_on(make_rtu_tcp_server(store));
    let (sink, stream) = rt.block_on(make_rtu_tcp_client(addr));
    let sink = RefCell::new(sink);
    let stream = RefCell::new(stream);
    let values: Vec<u16> = (0..10).collect();

    c.bench_function("rtu_tcp_write_multiple_registers_latency", |b| {
        let sink = &sink;
        let stream = &stream;
        b.to_async(&rt).iter(|| {
            let frame = write_multiple_registers_rtu(1, 0, &values);
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
    bench_rtu_tcp_read_holding_registers_latency,
    bench_rtu_tcp_write_single_register_latency,
    bench_rtu_tcp_read_coils_latency,
    bench_rtu_tcp_write_multiple_registers_latency,
);
criterion_main!(benches);
