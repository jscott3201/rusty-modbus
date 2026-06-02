# Rusty Modbus Benchmark Report

Last updated: 2026-06-02

This document records the current local and Docker performance baseline for the
Modbus/TCP client/server path. The focus is single-connection pipelining: one
TCP client connection, multiple concurrent in-flight requests, and a loopback
server backed by the in-memory store.

## Environment

| Item | Value |
|---|---|
| Git commit | `97c7413` base plus this direct FC08 diagnostics response refresh |
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

Codec/framing microbenchmarks are run with:

```bash
scripts/bench-local.sh codec --quick --noplot
scripts/bench-local.sh store --quick --noplot
scripts/bench-local.sh handler --quick --noplot
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

## Codec and Zero-Copy Direction

The current codec already uses the most important zero-copy pattern for Modbus:
decode operates over caller-owned `&[u8]`, variable-length response payloads
borrow from that buffer, and owned response types slice `bytes::Bytes` instead of
copying payloads.

On the server side, the in-memory store now exposes direct wire-byte hooks for
FC 0x01/0x02 bit tables, FC 0x03/0x04 register tables, FC 0x14/0x15 file
records, and FC 0x18 FIFO queues. The handler allocates the final response PDU
once and lets the store write directly into the wire payload bytes, avoiding the
previous scratch buffers, queue clone, per-group `Vec<u16>` materialization, and
second response-encoding pass on common paths.

FC 0x2B / MEI 0x0E Read Device Identification now keeps the configured object
list on the stack, slices basic/regular selections without temporary vectors,
and pre-sizes the final response buffer.

FC 0x11 Report Server ID now lets direct-access stores append identification
bytes into the final response buffer, avoiding the previous cloned server-id
blob before response encoding.

FC 0x08 Diagnostics now lets stores append response data into the final
response buffer. The in-memory store uses this to echo Return Query Data from
borrowed request bytes instead of cloning the diagnostic payload first.

`zerocopy` is already used where it is a strong fit: the fixed 7-byte MBAP
header is represented as a packed, network-endian wire-format type and the frame
decoder overlays it onto the read buffer before slicing the PDU. The benchmark
suite now includes MBAP decode with per-iteration allocation and MBAP decode
with a reused receive buffer so future changes can distinguish parser cost from
buffer allocation cost.

The PDU codec remains hand-written for now. Most PDU decode paths read a few
big-endian `u16` fields and then borrow the remaining payload. Extending
`zerocopy` into every small request/response body would add layout types and
derive requirements without an obvious copy to remove. `rkyv` is not a fit for
the Modbus wire path: it is designed for data serialized into rkyv's archived
layout, while Modbus is an external big-endian protocol format with per-function
validation rules.

The codec quick smoke was run with `scripts/bench-local.sh codec --quick --noplot`.
Treat these as hotspot-shape indicators, not release-grade Criterion baselines:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| Max FC 0x10 request decode | 1.58 ns | Decode validates the envelope and borrows payload bytes. |
| Max FC 0x03 response decode | 1.53 ns | Response decode borrows register payload bytes. |
| Max FC 0x03 response decode + register iteration | 44.6 ns | Register value access, not decode, is the first payload-sized cost. |
| Owned `Bytes` FC 0x03 dispatch | 12.3 ns | Owned slicing/refcount path is still small. |
| MBAP decode, fresh buffer per iteration | 31.6 ns | Includes receive-buffer allocation/copy shape. |
| MBAP decode, reused buffer | 13.9 ns | Isolates framing/parser work more closely. |
| Max register write unpack to `Vec<u16>` | 63.1 ns | Server write materialization is larger than decode. |
| Max coil write unpack to `Vec<bool>` | 377.5 ns | Packed-bit expansion is the strongest current allocation/copy candidate. |

The packed store read/write quick smoke was run with
`scripts/bench-local.sh store --quick --noplot` after adding direct wire-byte
paths to the in-memory store:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| Max register write from `&[u16]` | 11.9 ns | Slice baseline for existing store API. |
| Max register write from wire bytes | 12.3 ns | Direct packed path avoids the previous temporary `Vec<u16>`. |
| Max register wire bytes via `Vec<u16>` | 112 ns | Approximate old handler shape. |
| Max register read to BE wire bytes | 51.2 ns | Store writes directly into the response payload buffer. |
| Max register read via `u16` buffer then pack | 65.8 ns | Approximate old handler shape; extra scratch copy/encode pass costs ~22%. |
| Max coil write from `&[bool]` | 43.3 ns | Slice baseline for existing store API. |
| Max coil write from packed wire bytes | 1.24 us | Direct packed path avoids the previous temporary `Vec<bool>`. |
| Max coil packed bytes via `Vec<bool>` | 1.27 us | Approximate old handler shape; packed-bit expansion dominates. |
| Max coil read to packed wire bytes | 1.42 us | Store writes directly into the response payload buffer. |
| Max coil read via bool buffer then pack | 1.53 us | Approximate old handler shape; extra scratch fill/copy/pack pass costs ~7%. |
| Max FIFO read to BE wire bytes | 26.3 ns | Store writes the queue snapshot directly into the response payload buffer. |
| Max FIFO read via cloned `Vec<u16>` then pack | 50.9 ns | Approximate old handler shape; queue clone and second pack pass roughly double this microbench. |
| Max file-record read to BE wire bytes | 59.1 ns | Store writes a 122-register file sub-record directly into the response payload buffer. |
| Max file-record read via `u16` buffer then pack | 70.9 ns | Approximate old FC14 handler shape; extra scratch copy/encode pass costs ~17%. |
| Max file-record write from `&[u16]` | 22.2 ns | Slice baseline for existing store API. |
| Max file-record write from wire bytes | 19.0 ns | Direct FC15 path keeps borrowed request bytes through validation and avoids per-group allocation. |
| Max file-record wire bytes via `Vec<u16>` | 117 ns | Approximate old FC15 handler shape; per-group vector materialization dominates. |

The RTU-over-TCP CRC scan quick smoke was run with
`scripts/bench-local.sh codec rtu_tcp --quick --noplot` after changing the
frame-boundary scan to update CRC state incrementally:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| RTU/TCP FC 0x03 read request decode | 30.2 ns | Short-frame happy path remains tiny. |
| RTU/TCP max-size valid frame decode | 430 ns | Full-frame scan stays sub-microsecond. |
| RTU/TCP full corrupt buffer decode | 382 ns | No-match path now scans once instead of rehashing every prefix. |
| Old-style prefix rescan, full corrupt buffer | 40.0 us | Benchmark-only comparator for the previous scan strategy. |

The server handler quick smoke was run with
`scripts/bench-local.sh handler --quick --noplot` after adding direct
`process_request` baselines. These rows include request decode, protocol
validation, in-memory store access, and response construction, but exclude TCP,
TLS, RTU framing, and client-side work:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| FC01 max coil read | 799 ns | Packed-bit response generation remains the largest measured handler path. |
| FC03 max holding-register read | 29.4 ns | Direct BE register response path keeps full-size reads small. |
| FC10 max register write | 28.3 ns | Direct BE write path avoids the old request-payload `Vec<u16>`. |
| FC14 two-group file read | 68.6 ns | Direct file-record response writes keep multi-group reads sub-100 ns. |
| FC17 max read/write registers | 43.7 ns | Read half now writes directly into the final response bytes. |
| FC18 FIFO two-value read | 26.5 ns | Direct FIFO response path is comparable to simple register handlers. |
| FC08 return query data | 21.1 ns | Direct diagnostic append path echoes borrowed request bytes into the response. |
| FC11 report server ID | 34.3 ns | Direct server-id append path avoids cloning the store blob before response construction. |
| FC2B basic device identification | 47.3 ns | Stack-backed object selection removes the previous object/filter/selection vectors. |

The most likely next performance wins are adjacent to, not inside, raw PDU
parsing:

- Keep Criterion baselines around maximum-size request decode, response
  dispatch, owned `Bytes` dispatch, register iteration, packed write paths, and
  server handler dispatch before changing parser internals.
- Continue evaluating diagnostics and device-identification paths where
  temporary vectors or store cloning still dominate more than borrowed decode.
- Add multi-client stress matrices to separate protocol overhead from Tokio task
  scheduling and connection scaling.
- Add allocation profiling for server handlers and Python bindings so zero-copy
  decisions target measured heap churn instead of parser aesthetics.

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
- Allocation profiling for server write handlers that currently unpack request
  payloads into temporary vectors before calling the datastore.
- Machine-readable benchmark history so future PRs can compare against this
  baseline automatically.
