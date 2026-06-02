# Rusty Modbus Benchmark Report

Last updated: 2026-06-02

This document records the current local and Docker performance baseline for the
Modbus/TCP client/server path. The focus is single-connection pipelining: one
TCP client connection, multiple concurrent in-flight requests, and a loopback
server backed by the in-memory store.

## Environment

| Item | Value |
|---|---|
| Git commit | `6fd41df` base plus this Docker benchmark-suite refresh |
| Host | Apple M5 class MacBook Pro, arm64 |
| OS | macOS 26.5.0 / Darwin 25.5.0 / arm64 |
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Cargo | `cargo 1.95.0 (f2d3ce0bd 2026-03-21)` |
| Docker | `Docker version 29.5.2, build 79eb04c` |
| Local build mode | `cargo run --release` |
| Docker image | Alpine 3.22 runtime and distroless static-debian12:nonroot runtime, Rust 1.95.0 Alpine builder |
| Transport | Modbus/TCP over loopback |
| Server | Spawned benchmark server, `InMemoryStore` |
| Workload duration | 1s warmup + 5s measured per row |
| Client shape | 1 client connection, varied in-flight depth |
| Register count | 10 registers per read operation |

## Commands

The comparable local + Docker matrix was run with:

```bash
scripts/bench-suite.sh all \
  --duration 5 \
  --warmup 1 \
  --clients 1 \
  --depths 1,2,4,8,16 \
  --operations read,mixed \
  --output-dir bench-output/stress-20260602-docker-local-suite
```

The same script can run either side independently:

```bash
scripts/bench-suite.sh local
scripts/bench-suite.sh docker
```

The local stress script now runs the stress binary in release mode by default:

```bash
cargo run --release -p rusty-modbus-benchmarks --bin stress-test -- ...
```

## Results

### Read Holding Registers

Workload: repeated FC 0x03 reads of 10 holding registers.

| Runtime | In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Local release | 1 | 57,788 | 288,939 | 0.016 | 0.021 | 0.031 | 0.036 | 0.087 | 0 | 0 |
| Local release | 2 | 102,866 | 514,331 | 0.018 | 0.028 | 0.033 | 0.040 | 0.093 | 0 | 0 |
| Local release | 4 | 167,054 | 835,268 | 0.023 | 0.032 | 0.040 | 0.066 | 0.741 | 0 | 1 |
| Local release | 8 | 250,918 | 1,254,591 | 0.031 | 0.043 | 0.050 | 0.064 | 0.126 | 0 | 1 |
| Local release | 16 | 288,162 | 1,440,810 | 0.055 | 0.079 | 0.091 | 0.114 | 0.316 | 0 | 1 |
| Alpine container | 1 | 101,274 | 506,368 | 0.008 | 0.024 | 0.025 | 0.030 | 0.519 | 0 | 0 |
| Alpine container | 2 | 153,820 | 769,098 | 0.011 | 0.022 | 0.029 | 0.037 | 0.109 | 0 | 0 |
| Alpine container | 4 | 176,964 | 884,821 | 0.023 | 0.030 | 0.038 | 0.044 | 0.166 | 0 | 0 |
| Alpine container | 8 | 250,919 | 1,254,596 | 0.031 | 0.045 | 0.052 | 0.061 | 0.091 | 0 | 0 |
| Alpine container | 16 | 288,122 | 1,440,611 | 0.052 | 0.082 | 0.092 | 0.103 | 0.188 | 0 | 0 |
| Distroless container | 1 | 96,004 | 480,020 | 0.008 | 0.024 | 0.025 | 0.030 | 0.362 | 0 | 0 |
| Distroless container | 2 | 155,690 | 778,448 | 0.011 | 0.022 | 0.029 | 0.035 | 0.125 | 0 | 0 |
| Distroless container | 4 | 176,732 | 883,661 | 0.023 | 0.030 | 0.037 | 0.044 | 0.076 | 0 | 0 |
| Distroless container | 8 | 252,216 | 1,261,081 | 0.031 | 0.045 | 0.052 | 0.061 | 0.112 | 0 | 0 |
| Distroless container | 16 | 288,198 | 1,440,988 | 0.052 | 0.082 | 0.092 | 0.103 | 0.728 | 0 | 0 |

### Mixed Read/Write

Workload: alternating FC 0x03 reads and FC 0x06 write-single-register requests.

| Runtime | In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Local release | 1 | 58,873 | 294,364 | 0.016 | 0.021 | 0.031 | 0.041 | 0.108 | 0 | 0 |
| Local release | 2 | 106,344 | 531,718 | 0.017 | 0.028 | 0.033 | 0.045 | 0.129 | 0 | 1 |
| Local release | 4 | 170,552 | 852,758 | 0.022 | 0.031 | 0.038 | 0.054 | 0.122 | 0 | 1 |
| Local release | 8 | 246,170 | 1,230,850 | 0.031 | 0.045 | 0.055 | 0.080 | 0.170 | 0 | 1 |
| Local release | 16 | 288,579 | 1,442,897 | 0.055 | 0.079 | 0.092 | 0.115 | 0.285 | 0 | 1 |
| Alpine container | 1 | 99,387 | 496,935 | 0.008 | 0.024 | 0.025 | 0.029 | 0.082 | 0 | 0 |
| Alpine container | 2 | 159,425 | 797,123 | 0.011 | 0.021 | 0.028 | 0.035 | 0.129 | 0 | 0 |
| Alpine container | 4 | 177,539 | 887,695 | 0.023 | 0.030 | 0.037 | 0.043 | 0.353 | 0 | 0 |
| Alpine container | 8 | 252,992 | 1,264,962 | 0.031 | 0.045 | 0.052 | 0.060 | 0.107 | 0 | 0 |
| Alpine container | 16 | 289,149 | 1,445,747 | 0.052 | 0.082 | 0.093 | 0.105 | 0.238 | 0 | 0 |
| Distroless container | 1 | 100,467 | 502,336 | 0.008 | 0.024 | 0.025 | 0.029 | 0.079 | 0 | 0 |
| Distroless container | 2 | 157,220 | 786,100 | 0.011 | 0.021 | 0.028 | 0.035 | 0.077 | 0 | 0 |
| Distroless container | 4 | 177,554 | 887,769 | 0.023 | 0.030 | 0.037 | 0.044 | 0.087 | 0 | 0 |
| Distroless container | 8 | 251,390 | 1,256,952 | 0.031 | 0.045 | 0.052 | 0.061 | 0.103 | 0 | 0 |
| Distroless container | 16 | 289,998 | 1,449,992 | 0.052 | 0.082 | 0.092 | 0.105 | 0.198 | 0 | 0 |

### Docker Image Footprint

These image sizes were collected with `docker inspect` after local arm64 builds.

| Image | Target | Size |
|---|---|---:|
| `rusty-modbus:local` | `runtime` | 6.7 MB |
| `rusty-modbus:distroless` | `distroless` | 2.9 MB |
| `rusty-modbus-bench:alpine` | `benchmark` | 8.9 MB |
| `rusty-modbus-bench:distroless` | `benchmark-distroless` | 5.2 MB |

## Findings

- Single-connection pipelining still scales materially on local loopback. The
  local release run scaled from 57.8k to 288.2k ops/sec for reads and from
  58.9k to 288.6k ops/sec for mixed read/write.
- All 30 local/Docker rows completed with zero request errors.
- Tail latency rose as expected with deeper queues, but p99 stayed below 0.1 ms
  for every local and Docker row in this matrix.
- RSS stayed effectively flat, with local measured deltas at 0-1 MiB across the
  matrix.
- The Docker runs are not an apples-to-apples replacement for native macOS
  numbers because Docker Desktop runs inside a Linux VM. In this environment the
  containers were faster at shallow queue depths, while depths 8 and 16
  converged with the local release run.
- The distroless runtime keeps the same functional smoke behavior as the Alpine
  runtime while cutting the local arm64 image footprint by roughly 56%.
- The distroless benchmark image is about 42% smaller than the Alpine benchmark
  image and showed no meaningful throughput penalty versus Alpine in this local
  loopback suite.
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
