# Rusty Modbus Benchmark Report

Last updated: 2026-06-02

This document records the current local performance baseline for the Modbus/TCP
client/server path. The focus is single-connection pipelining: one TCP client
connection, multiple concurrent in-flight requests, and a local loopback server
backed by the in-memory store. This refresh was run after the codec strict
fixed-length PDU validation change in `d5f2681`.

## Environment

| Item | Value |
|---|---|
| Git commit | `d5f2681` |
| Host | Apple M5 class MacBook Pro, arm64 |
| OS | macOS 26.5.0 / Darwin 25.5.0 / arm64 |
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Cargo | `cargo 1.95.0 (f2d3ce0bd 2026-03-21)` |
| Docker | `Docker version 29.5.2, build 79eb04c` |
| Local build mode | `cargo run --release` |
| Docker image | Alpine 3.22 runtime, Rust 1.95.0 Alpine builder |
| Transport | Modbus/TCP over loopback |
| Server | Spawned benchmark server, `InMemoryStore` |
| Workload duration | 1s warmup + 5s measured per row |
| Client shape | 1 client connection, varied in-flight depth |
| Register count | 10 registers per read operation |

## Commands

The local benchmark matrix was run with:

```bash
for op in read mixed; do
  for depth in 1 2 4 8 16; do
    scripts/bench-local.sh stress \
      --duration 5 \
      --warmup 1 \
      --clients 1 \
      --in-flight "$depth" \
      --operation "$op" \
      --json
  done
done
```

The Docker benchmark target was run with:

```bash
scripts/docker-bench.sh \
  --duration 5 \
  --warmup 1 \
  --clients 1 \
  --in-flight 8 \
  --operation mixed \
  --json
```

The local stress script now runs the stress binary in release mode by default:

```bash
cargo run --release -p rusty-modbus-benchmarks --bin stress-test -- ...
```

## Results

### Read Holding Registers

Workload: repeated FC 0x03 reads of 10 holding registers.

| In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 58,025 | 290,125 | 0.016 | 0.021 | 0.031 | 0.041 | 0.121 | 0 | 0 |
| 2 | 103,923 | 519,613 | 0.018 | 0.028 | 0.034 | 0.049 | 0.104 | 0 | 1 |
| 4 | 167,587 | 837,934 | 0.023 | 0.032 | 0.039 | 0.053 | 0.124 | 0 | 1 |
| 8 | 246,526 | 1,232,628 | 0.031 | 0.045 | 0.053 | 0.069 | 0.133 | 0 | 1 |
| 16 | 287,869 | 1,439,344 | 0.055 | 0.080 | 0.092 | 0.118 | 0.203 | 0 | 1 |

### Mixed Read/Write

Workload: alternating FC 0x03 reads and FC 0x06 write-single-register requests.

| In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 59,099 | 295,494 | 0.016 | 0.021 | 0.031 | 0.040 | 0.220 | 0 | 0 |
| 2 | 106,730 | 533,648 | 0.017 | 0.028 | 0.034 | 0.043 | 0.117 | 0 | 1 |
| 4 | 170,655 | 853,273 | 0.022 | 0.031 | 0.038 | 0.050 | 0.132 | 0 | 1 |
| 8 | 244,123 | 1,220,614 | 0.031 | 0.046 | 0.059 | 0.095 | 0.183 | 0 | 1 |
| 16 | 290,026 | 1,450,130 | 0.055 | 0.079 | 0.091 | 0.115 | 0.301 | 0 | 1 |

### Docker Benchmark Target

Workload: the benchmark image running the mixed workload at in-flight depth 8.

| Runtime | In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Alpine container | 8 | 253,392 | 1,266,958 | 0.030 | 0.045 | 0.051 | 0.061 | 0.104 | 0 | 0 |

## Findings

- Single-connection pipelining still scales materially on loopback. Depth 16
  delivered about 4.96x the depth-1 read throughput and about 4.91x the depth-1
  mixed throughput.
- All local and Docker rows completed with zero request errors.
- Tail latency rose as expected with deeper queues, but local p99 stayed below
  0.1 ms for both workloads. Local p99.9 stayed at or below 0.118 ms.
- RSS stayed effectively flat, with local measured deltas at 0-1 MiB across the
  matrix.
- The Docker benchmark target is in the same range as the local release run. The
  depth-8 mixed Docker row measured about 3.8% above the local depth-8 mixed row,
  which should be treated as local-run variance rather than a container
  advantage.
- No throughput regression is visible from the recent strict codec validation
  changes; the refreshed numbers are close to the previous `4e88718` baseline.

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
- The stress benchmark spawns a loopback server; restricted sandboxes may need
  explicit permission to bind local sockets.

## Next Benchmarks

- TCP/TLS/RTU-over-TCP comparison for read, write, and mixed workloads.
- Multi-client plus per-client in-flight matrix to separate connection scaling
  from single-connection pipelining.
- Python binding throughput against a Python baseline.
- Codec-only Criterion baselines for owned response decoding and MBAP frame
  encode/decode before touching hot-path internals.
- Machine-readable benchmark history so future PRs can compare against this
  baseline automatically.
