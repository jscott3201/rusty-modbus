//! Sustained load stress test for Modbus transports.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use futures_util::stream::{FuturesUnordered, StreamExt};
use hdrhistogram::Histogram;
use rusty_modbus_benchmarks::frame_builders;
use rusty_modbus_benchmarks::helpers::{
    current_rss_bytes, make_store, make_tcp_client, make_tcp_server_with_store,
};
use rusty_modbus_benchmarks::rtu_helpers::{make_rtu_tcp_client, make_rtu_tcp_server};
use rusty_modbus_benchmarks::tls_helpers::{generate_test_certs, make_tls_client, make_tls_server};
use rusty_modbus_client::{ClientError, ModbusClient};
use rusty_modbus_tcp::transport::{TransportSink, TransportStream};
use rusty_modbus_types::UnitId;
use serde::Serialize;
use tokio::task::JoinHandle;

#[derive(Parser)]
#[command(name = "stress-test", about = "Modbus sustained load stress test")]
struct Args {
    /// Transport to test: tcp, tls, rtu-tcp
    #[arg(long, default_value = "tcp")]
    transport: String,

    /// External server address (default: spawn loopback)
    #[arg(long)]
    target: Option<String>,

    /// Test duration in seconds
    #[arg(long, default_value = "60")]
    duration: u64,

    /// Number of concurrent clients
    #[arg(long, default_value = "1")]
    clients: usize,

    /// Workload type: read, write, mixed
    #[arg(long, default_value = "mixed")]
    operation: String,

    /// Registers per read/write operation
    #[arg(long, default_value = "10")]
    registers: u16,

    /// Concurrent in-flight requests per TCP client connection (1..16)
    #[arg(long, default_value_t = 1, value_parser = parse_in_flight)]
    in_flight: usize,

    /// Output as JSON
    #[arg(long)]
    json: bool,

    /// Warmup period in seconds
    #[arg(long, default_value = "5")]
    warmup: u64,
}

#[derive(Serialize)]
struct StressResult {
    schema_version: u32,
    transport: String,
    clients: usize,
    in_flight: usize,
    duration_secs: u64,
    warmup_secs: u64,
    operation: String,
    registers: u16,
    total_ops: u64,
    throughput_ops_sec: f64,
    per_client_ops_sec: f64,
    latency_ms: LatencyStats,
    errors: u64,
    error_rate: f64,
    /// The TCP helper sets max retries to zero; the TLS and RTU loops have no retry layer.
    retry_attempts: u64,
    memory: MemoryStats,
}

#[derive(Serialize)]
struct LatencyStats {
    p50: f64,
    p95: f64,
    p99: f64,
    p999: f64,
    min: f64,
    max: f64,
}

#[derive(Serialize)]
struct MemoryStats {
    rss_before_mb: u64,
    rss_after_mb: u64,
    delta_mb: i64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let store = make_store();
    let running = Arc::new(AtomicBool::new(true));
    let rss_before = current_rss_bytes();

    // Spawn server if no external target.
    let server_addr: SocketAddr = if let Some(ref target) = args.target {
        target.parse().expect("invalid target address")
    } else {
        match args.transport.as_str() {
            "tcp" => {
                let (server, addr) = make_tcp_server_with_store(Arc::clone(&store)).await;
                // Keep server alive for the process lifetime.
                std::mem::forget(server);
                addr
            }
            "tls" => {
                let certs = generate_test_certs();
                let (handle, addr) = make_tls_server(&certs, Arc::clone(&store)).await;
                std::mem::forget(certs);
                std::mem::forget(handle);
                addr
            }
            "rtu-tcp" => {
                let (handle, addr) = make_rtu_tcp_server(Arc::clone(&store)).await;
                std::mem::forget(handle);
                addr
            }
            other => {
                eprintln!("Unknown transport: {other}. Use tcp, tls, or rtu-tcp.");
                std::process::exit(1);
            }
        }
    };

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Warmup phase.
    if args.warmup > 0 {
        eprintln!("Warming up for {}s...", args.warmup);
        let warmup_running = Arc::new(AtomicBool::new(true));
        let wr = Arc::clone(&warmup_running);

        let handle = spawn_client_task(
            &args.transport,
            server_addr,
            &args.operation,
            args.registers,
            args.in_flight,
            Arc::clone(&wr),
        )
        .await;

        tokio::time::sleep(Duration::from_secs(args.warmup)).await;
        wr.store(false, Ordering::Relaxed);
        let _ = handle.await;
        eprintln!("Warmup complete.");
    }

    // Measurement phase.
    eprintln!(
        "Running {} clients for {}s ({} workload, {} transport)...",
        args.clients, args.duration, args.operation, args.transport
    );

    let mut handles = Vec::new();
    for _ in 0..args.clients {
        let handle = spawn_client_task(
            &args.transport,
            server_addr,
            &args.operation,
            args.registers,
            args.in_flight,
            Arc::clone(&running),
        )
        .await;
        handles.push(handle);
    }

    tokio::time::sleep(Duration::from_secs(args.duration)).await;
    running.store(false, Ordering::Relaxed);

    // Collect results.
    let mut merged = Histogram::<u64>::new(3).unwrap();
    let mut total_ops: u64 = 0;
    let mut total_errors: u64 = 0;

    for handle in handles {
        let (hist, ops, errors) = handle.await.unwrap();
        merged.add(&hist).unwrap();
        total_ops += ops;
        total_errors += errors;
    }

    let rss_after = current_rss_bytes();

    let result = StressResult {
        schema_version: 1,
        transport: args.transport.clone(),
        clients: args.clients,
        in_flight: args.in_flight,
        duration_secs: args.duration,
        warmup_secs: args.warmup,
        operation: args.operation.clone(),
        registers: args.registers,
        total_ops,
        throughput_ops_sec: total_ops as f64 / args.duration as f64,
        per_client_ops_sec: total_ops as f64 / args.duration as f64 / args.clients as f64,
        latency_ms: LatencyStats {
            p50: merged.value_at_quantile(0.5) as f64 / 1000.0,
            p95: merged.value_at_quantile(0.95) as f64 / 1000.0,
            p99: merged.value_at_quantile(0.99) as f64 / 1000.0,
            p999: merged.value_at_quantile(0.999) as f64 / 1000.0,
            min: merged.min() as f64 / 1000.0,
            max: merged.max() as f64 / 1000.0,
        },
        errors: total_errors,
        error_rate: if total_ops + total_errors > 0 {
            total_errors as f64 / (total_ops + total_errors) as f64
        } else {
            0.0
        },
        retry_attempts: 0,
        memory: MemoryStats {
            rss_before_mb: rss_before / (1024 * 1024),
            rss_after_mb: rss_after / (1024 * 1024),
            delta_mb: (rss_after as i64 - rss_before as i64) / (1024 * 1024),
        },
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        println!(
            "Transport: {} | Clients: {} | Duration: {}s | Operation: {}\n",
            result.transport, result.clients, result.duration_secs, result.operation
        );
        println!(
            "In-flight:   {} request(s) per TCP client connection",
            result.in_flight
        );
        println!(
            "Throughput:  {:.0} ops/sec (total)  |  {:.0} ops/sec (per client)",
            result.throughput_ops_sec, result.per_client_ops_sec
        );
        println!(
            "Latency:     p50={:.2}ms  p95={:.2}ms  p99={:.2}ms  p99.9={:.2}ms",
            result.latency_ms.p50,
            result.latency_ms.p95,
            result.latency_ms.p99,
            result.latency_ms.p999
        );
        println!(
            "Errors:      {} ({:.2}%)",
            result.errors,
            result.error_rate * 100.0
        );
        println!(
            "Memory:      RSS before={}MB  after={}MB  delta={:+}MB",
            result.memory.rss_before_mb, result.memory.rss_after_mb, result.memory.delta_mb
        );
    }
}

async fn spawn_client_task(
    transport: &str,
    addr: SocketAddr,
    operation: &str,
    registers: u16,
    in_flight: usize,
    running: Arc<AtomicBool>,
) -> JoinHandle<(Histogram<u64>, u64, u64)> {
    let transport = transport.to_string();
    let operation = operation.to_string();

    tokio::spawn(async move {
        let mut hist = Histogram::<u64>::new(3).unwrap();
        let mut ops: u64 = 0;
        let mut errors: u64 = 0;
        let mut op_index: u64 = 0;

        match transport.as_str() {
            "tcp" => {
                let client = make_tcp_client(addr).await;
                let mut pending = FuturesUnordered::new();
                while running.load(Ordering::Relaxed) || !pending.is_empty() {
                    while running.load(Ordering::Relaxed) && pending.len() < in_flight {
                        let start = Instant::now();
                        let index = op_index;
                        let client = &client;
                        let operation = operation.as_str();
                        pending.push(async move {
                            let result =
                                run_tcp_operation(client, operation, registers, index).await;
                            (start.elapsed().as_micros() as u64, result)
                        });
                        op_index += 1;
                    }

                    let Some((elapsed_us, result)) = pending.next().await else {
                        break;
                    };
                    match result {
                        Ok(()) => {
                            let _ = hist.record(elapsed_us);
                            ops += 1;
                        }
                        Err(_) => errors += 1,
                    }
                }
                client.shutdown().await;
            }
            "tls" => {
                let certs = generate_test_certs();
                let (mut sink, mut stream) = make_tls_client(&certs, addr).await;
                let mut txn_id = 0u16;

                while running.load(Ordering::Relaxed) {
                    txn_id = txn_id.wrapping_add(1);
                    let start = Instant::now();
                    let frame = match operation.as_str() {
                        "read" => {
                            frame_builders::read_holding_registers_mbap(txn_id, 1, 0, registers)
                        }
                        "write" => frame_builders::write_single_register_mbap(
                            txn_id,
                            1,
                            (op_index % 100) as u16,
                            0x1234,
                        ),
                        _ => {
                            if op_index.is_multiple_of(2) {
                                frame_builders::read_holding_registers_mbap(txn_id, 1, 0, registers)
                            } else {
                                frame_builders::write_single_register_mbap(
                                    txn_id,
                                    1,
                                    (op_index % 100) as u16,
                                    0x1234,
                                )
                            }
                        }
                    };
                    match sink.send(frame).await {
                        Ok(()) => match stream.recv().await {
                            Ok(_) => {
                                let elapsed_us = start.elapsed().as_micros() as u64;
                                let _ = hist.record(elapsed_us);
                                ops += 1;
                            }
                            Err(_) => errors += 1,
                        },
                        Err(_) => errors += 1,
                    }
                    op_index += 1;
                }
            }
            "rtu-tcp" => {
                let (mut sink, mut stream) = make_rtu_tcp_client(addr).await;

                while running.load(Ordering::Relaxed) {
                    let start = Instant::now();
                    let frame = match operation.as_str() {
                        "read" => frame_builders::read_holding_registers_rtu(1, 0, registers),
                        "write" => frame_builders::write_single_register_rtu(
                            1,
                            (op_index % 100) as u16,
                            0x1234,
                        ),
                        _ => {
                            if op_index.is_multiple_of(2) {
                                frame_builders::read_holding_registers_rtu(1, 0, registers)
                            } else {
                                frame_builders::write_single_register_rtu(
                                    1,
                                    (op_index % 100) as u16,
                                    0x1234,
                                )
                            }
                        }
                    };
                    match sink.send(frame).await {
                        Ok(()) => match stream.recv().await {
                            Ok(_) => {
                                let elapsed_us = start.elapsed().as_micros() as u64;
                                let _ = hist.record(elapsed_us);
                                ops += 1;
                            }
                            Err(_) => errors += 1,
                        },
                        Err(_) => errors += 1,
                    }
                    op_index += 1;
                }
            }
            _ => {}
        }

        (hist, ops, errors)
    })
}

async fn run_tcp_operation(
    client: &ModbusClient,
    operation: &str,
    registers: u16,
    op_index: u64,
) -> Result<(), ClientError> {
    match operation {
        "read" => client
            .read_holding_registers(UnitId(1), 0, registers)
            .await
            .map(|_| ()),
        "write" => {
            client
                .write_single_register(UnitId(1), (op_index % 100) as u16, 0x1234)
                .await
        }
        _ if op_index.is_multiple_of(2) => client
            .read_holding_registers(UnitId(1), 0, registers)
            .await
            .map(|_| ()),
        _ => {
            client
                .write_single_register(UnitId(1), (op_index % 100) as u16, 0x1234)
                .await
        }
    }
}

fn parse_in_flight(raw: &str) -> Result<usize, String> {
    let value = raw
        .parse::<usize>()
        .map_err(|err| format!("invalid in-flight depth: {err}"))?;
    if (1..=16).contains(&value) {
        Ok(value)
    } else {
        Err(String::from("in-flight depth must be between 1 and 16"))
    }
}

#[cfg(test)]
mod tests {
    use super::{LatencyStats, MemoryStats, StressResult, parse_in_flight};

    #[test]
    fn in_flight_parser_accepts_transaction_ring_bounds() {
        assert_eq!(parse_in_flight("1").unwrap(), 1);
        assert_eq!(parse_in_flight("16").unwrap(), 16);
    }

    #[test]
    fn in_flight_parser_rejects_out_of_range_values() {
        assert!(parse_in_flight("0").is_err());
        assert!(parse_in_flight("17").is_err());
    }

    #[test]
    fn json_contract_records_schema_workload_and_no_retries() {
        let result = StressResult {
            schema_version: 1,
            transport: String::from("tcp"),
            clients: 1,
            in_flight: 8,
            duration_secs: 5,
            warmup_secs: 1,
            operation: String::from("read"),
            registers: 10,
            total_ops: 100,
            throughput_ops_sec: 20.0,
            per_client_ops_sec: 20.0,
            latency_ms: LatencyStats {
                p50: 0.1,
                p95: 0.2,
                p99: 0.3,
                p999: 0.4,
                min: 0.05,
                max: 0.5,
            },
            errors: 0,
            error_rate: 0.0,
            retry_attempts: 0,
            memory: MemoryStats {
                rss_before_mb: 10,
                rss_after_mb: 11,
                delta_mb: 1,
            },
        };
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["warmup_secs"], 1);
        assert_eq!(value["registers"], 10);
        assert_eq!(value["retry_attempts"], 0);
    }
}
