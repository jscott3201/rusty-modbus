# Rusty Modbus Benchmark Report

Last updated: 2026-06-02

This document records the current local performance baseline for the Modbus/TCP
client/server path. The focus of this run is single-connection pipelining: one
TCP client connection, multiple concurrent in-flight requests, and a local
loopback server backed by the in-memory store.

## Environment

| Item | Value |
|---|---|
| Git commit | `7a49383` |
| Host | Apple M5, 10 cores, 16 GiB memory |
| OS | macOS 26.5.0 / Darwin 25.5.0 / arm64 |
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Cargo | `cargo 1.95.0 (f2d3ce0bd 2026-03-21)` |
| Build mode | `--release` |
| Transport | Modbus/TCP over loopback |
| Server | Spawned benchmark server, `InMemoryStore` |
| Workload duration | 1s warmup + 5s measured per row |
| Client shape | 1 client connection, varied in-flight depth |
| Register count | 10 registers per read operation |

## Commands

The benchmark matrix was run with:

```bash
for op in read mixed; do
  for depth in 1 2 4 8 16; do
    cargo run --release -p rusty-modbus-benchmarks --bin stress-test -- \
      --duration 5 \
      --warmup 1 \
      --clients 1 \
      --in-flight "$depth" \
      --operation "$op" \
      --json
  done
done
```

The benchmark wiring was also smoke-tested with:

```bash
scripts/bench-local.sh smoke
```

## Results

### Read Holding Registers

Workload: repeated FC 0x03 reads of 10 holding registers.

| In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 57,483 | 287,417 | 0.016 | 0.023 | 0.032 | 0.052 | 1.605 | 0 | 0 |
| 2 | 99,567 | 497,837 | 0.018 | 0.030 | 0.047 | 0.084 | 1.607 | 0 | 1 |
| 4 | 165,006 | 825,031 | 0.023 | 0.032 | 0.040 | 0.066 | 0.180 | 0 | 1 |
| 8 | 239,180 | 1,195,900 | 0.031 | 0.048 | 0.067 | 0.111 | 0.329 | 0 | 1 |
| 16 | 284,508 | 1,422,538 | 0.055 | 0.081 | 0.095 | 0.135 | 0.261 | 0 | 1 |

### Mixed Read/Write

Workload: alternating FC 0x03 reads and FC 0x06 write-single-register requests.

| In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 58,630 | 293,151 | 0.016 | 0.022 | 0.031 | 0.043 | 0.099 | 0 | 0 |
| 2 | 105,098 | 525,489 | 0.017 | 0.028 | 0.036 | 0.064 | 0.444 | 0 | 1 |
| 4 | 170,311 | 851,554 | 0.022 | 0.031 | 0.038 | 0.060 | 0.139 | 0 | 1 |
| 8 | 247,621 | 1,238,104 | 0.031 | 0.044 | 0.052 | 0.068 | 0.152 | 0 | 1 |
| 16 | 289,059 | 1,445,295 | 0.055 | 0.079 | 0.091 | 0.116 | 0.199 | 0 | 1 |

## Findings

- Single-connection pipelining scales materially on loopback. Depth 16 delivered
  about 4.95x the depth-1 read throughput and about 4.93x the depth-1 mixed
  throughput.
- All rows completed with zero request errors.
- Tail latency increased as expected with deeper queues, but p99 stayed below
  0.1 ms for both workloads in this local loopback run.
- RSS stayed effectively flat, with the measured delta at 0-1 MiB across the
  matrix.
- Throughput gains flatten between depth 8 and 16. That is the first area to
  inspect when optimizing the client hot path, especially sink serialization,
  transaction registration, response matching, and server handler overhead.

## Caveats

- These numbers are local loopback measurements on one developer machine. They
  are useful as a regression baseline and hotspot guide, not as cross-machine
  marketing numbers.
- The server and client run on the same host, so kernel scheduling and loopback
  behavior dominate more than real network latency.
- The matrix uses an in-memory store. Device, gateway, TLS, serial, and slow-store
  workloads need separate baselines.
- The run uses 5-second measurement windows for timely iteration. Release-facing
  comparisons should use longer windows and Criterion baselines where practical.

## Next Benchmarks

- TCP/TLS comparison for read, write, and mixed workloads.
- Multi-client plus per-client in-flight matrix to separate connection scaling
  from single-connection pipelining.
- Python binding throughput against a Python baseline.
- Codec-only Criterion baselines for owned response decoding and MBAP frame
  encode/decode before touching hot-path internals.
