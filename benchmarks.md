# Rusty Modbus Benchmark Report

Last updated: 2026-08-31

This document records the current local and Docker performance baseline for the
Modbus/TCP client/server path. The focus is single-connection pipelining: one
TCP client connection, multiple concurrent in-flight requests, and a loopback
server backed by the in-memory store.

## Environment

| Item | Value |
|---|---|
| Git commit | `e776964` on `dev` |
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

## Reproducible baseline harness

`scripts/baseline.py` records and validates correctness commands and benchmark
samples. It uses only the Python standard library. Its three run modes are:

- `correctness`: runs the formatting, ledger, harness, lint, test, feature,
  example, Python binding, supply-chain, and advisory checks as separate
  recorded commands. This mode requires `cargo-nextest`, `cargo-deny`,
  `cargo-audit`, Python 3.14, and `uv` in addition to the pinned Rust toolchain.
- `bench-smoke`: runs one-second TCP `read` and `mixed` loopback samples at
  in-flight depths 1, 8, and 16, followed by the pipelined TCP Criterion
  throughput rows and both `tcp_pool` lifecycle rows.
- `bench-full`: runs five TCP repetitions at depths 1, 2, 4, 8, and 16, then
  discovers and runs all registered `tcp_*` Criterion targets, including
  `tcp_pool`. Stress measurements default to five seconds with a one-second
  warmup. Codec, server-only, TLS, and RTU targets are outside both benchmark
  modes.

The throughput samples measure TCP loopback performance. The pool lifecycle
rows are narrower observations of the exact work described below, not transport
health or protocol-conformance evidence. The recorded commands in `correctness`
provide the correctness evidence. `.github/workflows/baseline.yml` runs on
Ubuntu only: pull requests and pushes run `bench-smoke`, the Monday schedule
runs `bench-full`, and manual runs select any mode.

Run a mode from the repository root and identify the runner:

```bash
python3 scripts/baseline.py correctness --runner-label local-workstation
python3 scripts/baseline.py bench-smoke --runner-label local-workstation
python3 scripts/baseline.py bench-full --runner-label local-workstation
```

### Uncontended TCP pool lifecycle target

Run both lifecycle rows directly with:

```bash
scripts/bench-local.sh tcp-pool --quick --noplot
```

Both `tcp_pool` rows use one benchmark task and borrower, a one-connection
non-priority pool, no capacity wait, no configured priority device, no
pre-connect, replenishment, or probe, and long idle/health intervals. There is
no concurrent pool activity or Modbus request. "Uncontended" does not exclude
OS or Tokio runtime scheduling.

- `tcp_pool/fresh_get_raw_drop` measures one public `pool.get(addr)` that opens
  a fresh loopback TCP connection, black-boxes only its public address, and
  drops the raw lease so it retires. It includes loopback TCP establishment and
  raw-lease drop; it is not pure pool overhead.
- `tcp_pool/reusable_checkout_handoff_shutdown_return` starts with one idle
  lease seeded outside timing. Each iteration checks it out, hands the pristine
  lease directly to a reusable client with a bounded zero-retry configuration,
  gracefully shuts down the client, recovers the transport, and returns it to
  idle. It includes reusable-client construction, child-task lifecycle,
  graceful shutdown, transport recovery, and idle return; it is not pure pool
  overhead.

Each row creates its Tokio runtime, loopback server, and pool outside its timed
loop. The reusable seed handoff/return is also outside timing. Custom batch
timing stops before exact return-outcome and pool-accounting assertions. Pool
shutdown, final accounting assertions, and server stop are cleanup outside
timing.

These rows and their reports are observational only. They define no threshold,
budget, improvement/regression label, comparison verdict, or accepted baseline;
the two intentionally different rows must not be compared with each other.
Results do not establish cross-run or cross-host comparability, liveness,
health, reconnect behavior, fairness, contention behavior, protocol behavior,
or any gateway, TLS, or RTU claim.

Because report comparison requires identical complete scenario-key sets, an
older report without `tcp_pool/*` keys fails closed when compared with a newer
report. Recollect both operands with identical target sets rather than partially
matching them.

Benchmark modes accept bounded `--duration`, `--warmup`, and `--repetitions`
overrides. All run modes accept `--output-root` and `--run-id`. The default
artifact path is:

```text
bench-output/baseline-v1/<full-40-character-SHA>/<run-id>/
├── environment.json
├── provenance.json
├── commands/<sequence>-<label>/{command.json,command.stdout,command.stderr}
├── stress/parsed/*.json                         # benchmark modes
├── criterion/{raw/**,parsed-estimates.json}     # benchmark modes
├── summary.json
├── summary.csv
├── benchmark-report-v1.json                    # successful benchmark modes
├── benchmark-report-v1.md                      # successful benchmark modes
└── checksums.sha256
```

Schema version `1` is defined in `scripts/baseline.py`. JSON files use sorted
keys and a trailing newline; the CSV has fixed columns. Command records contain
the exact argument array, working directory, UTC timing, exit code, and only
explicit environment overrides. Raw command output and Criterion data remain
the source evidence. `summary.json` and `summary.csv` are parsed views, not a
replacement for those files. Commands inherit the runner environment, so the
artifact is not a hermetic-environment record and does not capture arbitrary
inherited variables or secrets.

The harness binds the artifact to the full SHA from `git rev-parse HEAD`. It
refuses tracked changes and non-ignored untracked files, and it never overwrites
a run directory. Ignored files under `bench-output/` and ignored `.DS_Store`
files do not make the worktree dirty. `--allow-dirty` exists for local
diagnosis; the resulting summary has `status: invalid`, and `validate` rejects
it. A failed command or missing/malformed stress or Criterion output makes the
run fail, but the harness still attempts to write the partial summary and
checksums. TCP stress samples must report zero errors, zero error rate, and zero
retry attempts; the TCP benchmark helper configures zero retries.

`checksums.sha256` covers every retained artifact file except itself and stores
repository-relative paths in bytewise order. Verify a copied or retained run
with:

```bash
python3 scripts/baseline.py validate bench-output/baseline-v1/<SHA>/<run-id>
```

The checksums detect missing or corrupt retained files. They are not a signature
or attestation: rewriting files and regenerating `checksums.sha256` defeats that
check.

### Machine-readable benchmark report contract

Successful `bench-smoke` and `bench-full` finalization now writes
`benchmark-report-v1.json` and `benchmark-report-v1.md` before constructing the
checksum inventory. The workflow already uploads the complete run directory, so
no measured value controls whether these informational reports are uploaded.
Correctness artifacts do not contain TCP stress/Criterion scenarios and do not
produce benchmark reports.

The report uses the independent `benchmark-report` schema version `1`. This does
not bump, reinterpret, or make the report files mandatory for baseline artifact
schema version `1`; retained v1 artifacts without reports remain valid. Render a
report from an existing validated benchmark artifact into a new, repository-local
directory with placeholder paths as follows:

```bash
python3 scripts/baseline.py report \
  bench-output/baseline-v1/<SHA>/<run-id> \
  --output-dir <new-report-output-dir>
python3 scripts/baseline.py validate-report \
  <new-report-output-dir>/benchmark-report-v1.json
```

The render command validates the source artifact and its checksum inventory,
does not modify the source, rejects traversal and symlink output paths, and
refuses an existing output directory. Repeated rendering from identical source
bytes is byte-identical. The report preserves source timestamps rather than
generating a render timestamp.

Each report records the full target SHA, run ID, mode, source status, declared
runner label, recorded environment and tool identity, strict zero-error and
zero-retry facts, normalized stress/Criterion values, and source-relative raw
and checksum references. Producer records identify custom stress JSON schema v1
and the exact Criterion 0.5.1 `new/estimates.json` private-layout adapter. The
renderer obtains that version from `Cargo.lock` at the artifact's validated full
target SHA through Git object storage; unavailable, ambiguous, mismatched, or
unsupported lock evidence is rejected rather than inferred from the current
checkout. That Criterion layout is not presented as a stable upstream API.

Report evidence is explicitly `observational_only`: artifact validity may be
`valid`, while performance comparability and runner isolation remain
`not_proven`, and budget and statistical decisions remain `not_evaluated`.
The report schema itself defines no performance budget, threshold, verdict,
accepted baseline, host-isolation policy, or cross-run comparison. The report
renderer does not compute deltas. Checksums remain an integrity inventory, not a
signature or attestation.

### Observed benchmark report deltas

The independent `benchmark-comparison` schema version `1` consumes two complete,
validated `benchmark-report` v1 JSON files. The first operand is positionally
named `baseline` and the second is positionally named `candidate`; neither name
means that a report has been accepted, promoted, or approved. Emit the canonical
comparison JSON to standard output with:

```bash
python3 scripts/baseline.py compare-report \
  <BASELINE-benchmark-report-v1.json> \
  <CANDIDATE-benchmark-report-v1.json>
```

The command uses the same repository-contained, symlink-rejecting report loader
as `validate-report`, including the target-SHA Criterion identity proof. It does
not modify either input or create an output directory. The two reports must have
the exact `benchmark-report` v1 schema identity, identical producer records, and
the same run mode. Their complete scenario-key sets must be equal; missing,
extra, ambiguous, or duplicate keys are rejected rather than partially matched.

TCP stress keys contain `kind`, `producer_id`, transport, operation, in-flight
depth, clients, registers, repetitions, duration seconds, and warmup seconds.
Criterion keys contain `kind`, `producer_id`, and benchmark ID; duplicate
Criterion benchmark IDs are rejected even when their private source paths are
different. Paired metrics must have identical units and shapes, and Criterion
confidence levels must be exactly equal before point estimates are observed.

Each matched TCP scenario records only the two input means and signed
`candidate_minus_baseline` for throughput (`operations_per_second`) and p99
latency (`ms`). Each matched Criterion scenario records only the two mean point
estimates and the same signed subtraction in `ns`. The output preserves each
operand's full target SHA, run ID, mode, declared runner label and recorded
runner context, source artifact provenance, and producer records. It does not
require or infer runner or environment equality.

Comparison evidence remains fixed to `classification=observational_only`,
`performance_comparability=not_proven`, `runner_isolation=not_proven`,
`budget_decision=not_evaluated`, and
`statistical_significance=not_evaluated`. Schema v1 defines no percentage,
direction label, improvement or regression wording, threshold, pass/fail,
budget verdict, confidence inference, statistical test, accepted baseline, or
performance decision. It generates no timestamp. Scenario-key ordering,
sorted-key JSON, and a trailing newline make repeated rendering of identical
inputs byte-identical.

The measured report below remains the June 2026 baseline; the harness does not
replace those numbers until a clean, committed-SHA run is recorded.

## June 2026 report commands

The comparable local + Docker matrix was run with:

```bash
scripts/bench-suite.sh all \
  --duration 5 \
  --warmup 1 \
  --clients 1 \
  --depths 1,2,4,8,16 \
  --operations read,mixed \
  --output-dir bench-output/stress-20260603-full-suite
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
scripts/bench-local.sh tcp-pipelined --quick --noplot
scripts/bench-local.sh tcp-pool --quick --noplot
```

Criterion quick-mode rows are run through the individual script modes instead
of `scripts/bench-local.sh all --quick --noplot` because Cargo runs the library
bench harness first in package-wide mode, and that harness rejects Criterion's
`--quick` flag.

## Results

### Read Holding Registers

Workload: repeated FC 0x03 reads of 10 holding registers.

| Runtime | In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Local release | 1 | 62,096 | 310,480 | 0.015 | 0.021 | 0.031 | 0.047 | 0.109 | 0 | 0 |
| Local release | 2 | 120,735 | 603,675 | 0.015 | 0.024 | 0.031 | 0.052 | 0.096 | 0 | 1 |
| Local release | 4 | 159,272.6 | 796,363 | 0.021 | 0.047 | 0.069 | 0.099 | 0.978 | 0 | 1 |
| Local release | 8 | 200,458 | 1,002,290 | 0.035 | 0.073 | 0.103 | 0.158 | 3.329 | 0 | 1 |
| Local release | 16 | 230,327.6 | 1,151,638 | 0.064 | 0.113 | 0.147 | 0.204 | 3.519 | 0 | 1 |
| Alpine container | 1 | 62,492.2 | 312,461 | 0.008 | 0.055 | 0.089 | 0.126 | 0.216 | 0 | 0 |
| Alpine container | 2 | 74,327.2 | 371,636 | 0.017 | 0.072 | 0.107 | 0.147 | 0.277 | 0 | 0 |
| Alpine container | 4 | 90,633.6 | 453,168 | 0.037 | 0.102 | 0.137 | 0.190 | 0.775 | 0 | 0 |
| Alpine container | 8 | 125,461.8 | 627,309 | 0.058 | 0.127 | 0.165 | 0.217 | 0.514 | 0 | 0 |
| Alpine container | 16 | 275,310.2 | 1,376,551 | 0.054 | 0.089 | 0.122 | 0.177 | 0.640 | 0 | 0 |
| Distroless container | 1 | 94,829.6 | 474,148 | 0.008 | 0.024 | 0.026 | 0.032 | 11.967 | 0 | 0 |
| Distroless container | 2 | 158,864.6 | 794,323 | 0.011 | 0.022 | 0.029 | 0.039 | 0.142 | 0 | 0 |
| Distroless container | 4 | 178,032.4 | 890,162 | 0.023 | 0.031 | 0.039 | 0.050 | 0.144 | 0 | 0 |
| Distroless container | 8 | 247,732.4 | 1,238,662 | 0.031 | 0.046 | 0.054 | 0.065 | 0.294 | 0 | 0 |
| Distroless container | 16 | 286,203 | 1,431,015 | 0.053 | 0.083 | 0.094 | 0.108 | 0.241 | 0 | 0 |

### Mixed Read/Write

Workload: alternating FC 0x03 reads and FC 0x06 write-single-register requests.

| Runtime | In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Local release | 1 | 59,054 | 295,270 | 0.016 | 0.021 | 0.031 | 0.039 | 0.093 | 0 | 0 |
| Local release | 2 | 112,511.2 | 562,556 | 0.016 | 0.026 | 0.035 | 0.057 | 0.177 | 0 | 1 |
| Local release | 4 | 161,249.6 | 806,248 | 0.022 | 0.040 | 0.064 | 0.091 | 5.027 | 0 | 1 |
| Local release | 8 | 250,629.2 | 1,253,146 | 0.031 | 0.044 | 0.051 | 0.066 | 0.141 | 0 | 1 |
| Local release | 16 | 251,765 | 1,258,825 | 0.058 | 0.117 | 0.159 | 0.204 | 0.309 | 0 | 1 |
| Alpine container | 1 | 98,961.8 | 494,809 | 0.008 | 0.024 | 0.026 | 0.053 | 0.780 | 0 | 0 |
| Alpine container | 2 | 142,523.8 | 712,619 | 0.011 | 0.023 | 0.033 | 0.074 | 2.179 | 0 | 0 |
| Alpine container | 4 | 175,937.6 | 879,688 | 0.022 | 0.034 | 0.048 | 0.090 | 5.763 | 0 | 0 |
| Alpine container | 8 | 247,175.8 | 1,235,879 | 0.031 | 0.047 | 0.059 | 0.080 | 0.167 | 0 | 0 |
| Alpine container | 16 | 287,541.8 | 1,437,709 | 0.052 | 0.082 | 0.093 | 0.108 | 0.267 | 0 | 0 |
| Distroless container | 1 | 95,229.4 | 476,147 | 0.008 | 0.025 | 0.026 | 0.032 | 11.223 | 0 | 0 |
| Distroless container | 2 | 155,972 | 779,860 | 0.011 | 0.023 | 0.029 | 0.038 | 0.140 | 0 | 0 |
| Distroless container | 4 | 178,149.2 | 890,746 | 0.023 | 0.031 | 0.038 | 0.049 | 0.134 | 0 | 0 |
| Distroless container | 8 | 250,089.6 | 1,250,448 | 0.031 | 0.046 | 0.054 | 0.069 | 1.391 | 0 | 0 |
| Distroless container | 16 | 287,946.4 | 1,439,732 | 0.052 | 0.082 | 0.093 | 0.105 | 0.378 | 0 | 0 |

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
  local release run scaled from 62.1k to 230.3k ops/sec for reads and from
  59.1k to 251.8k ops/sec for mixed read/write.
- All 30 local/Docker rows completed with zero request errors.
- Tail latency rose as expected with deeper queues. p99 stayed below 0.17 ms
  for every local and Docker row in this matrix.
- RSS stayed effectively flat, with local measured deltas at 0-1 MiB across the
  matrix.
- The Docker runs are not an apples-to-apples replacement for native macOS
  numbers because Docker Desktop runs inside a Linux VM. In this environment
  distroless remained faster than native at most queue depths, Alpine lagged at
  depths 2-8 on the read-only workload, and both containers converged around
  286k-288k ops/sec at depth 16.
- The distroless runtime keeps the same functional smoke behavior as the Alpine
  runtime while cutting the local arm64 image footprint by roughly 56%.
- The distroless benchmark image is about 42% smaller than the Alpine benchmark
  image and was faster than Alpine on most rows in this local loopback suite.
- Docker Desktop produced isolated max-latency outliers at shallow distroless
  depth-1 rows, but the p99 and p99.9 values stayed low and no request errors
  were recorded.

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

FC 0x0C Get Comm Event Log now lets stores append bounded event bytes into the
final response buffer while returning only the fixed status/counter metadata.
Existing stores that return an owned `CommEventLog` still work through the
default hook.

FC 0x14 Read File Record now builds the final response PDU directly while each
sub-response group is filled, avoiding the previous intermediate response-data
buffer and second encode/copy pass.

FC 0x15 Write File Record now validates sub-requests into a fixed stack buffer
bounded by the protocol's 0xFB-byte request-data cap. A one-register write group
is the smallest valid sub-request at 9 bytes, so the largest valid request can
contain 27 groups; this preserves the two-pass "validate before commit" behavior
without a heap `Vec` for group staging.

Packed coil/discrete helpers now work a byte at a time instead of repeatedly
dividing each bit index back into an output byte. This keeps the wire format
unchanged while reducing the dominant FC 0x01/0x02 read cost and the FC 0x0F
packed write unpack path.

The in-memory store now keeps coil and discrete-input tables in byte-backed bit
tables instead of `Vec<bool>`. That gives direct Modbus wire-byte paths a
single packed representation to read/write, while the public bool-slice methods
now pay a pack/unpack boundary cost.

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
| Max FC 0x03 response decode | 1.48 ns | Response decode borrows register payload bytes. |
| Max FC 0x03 response decode + register iteration | 44.2 ns | Register value access, not decode, is the first payload-sized cost. |
| Owned `Bytes` FC 0x03 dispatch | 12.3 ns | Owned slicing/refcount path is still small. |
| MBAP decode, fresh buffer per iteration | 29.8 ns | Includes receive-buffer allocation/copy shape. |
| MBAP decode, reused buffer | 13.9 ns | Isolates framing/parser work more closely. |
| Max register write unpack to `Vec<u16>` | 62.5 ns | Server write materialization is larger than decode. |
| Max coil write unpack to `Vec<bool>` | 371 ns | Packed-bit expansion is the strongest current allocation/copy candidate. |

The packed store read/write quick smoke was run with
`scripts/bench-local.sh store --quick --noplot` after adding direct wire-byte
paths to the in-memory store:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| Max register write from `&[u16]` | 6.53 ns | Slice baseline for existing store API. |
| Max register write from wire bytes | 7.43 ns | Direct packed path avoids the previous temporary `Vec<u16>`. |
| Max register wire bytes via `Vec<u16>` | 68.7 ns | Approximate old handler shape. |
| Max register read to BE wire bytes | 28.9 ns | Store writes directly into the response payload buffer. |
| Max register read via `u16` buffer then pack | 37.9 ns | Approximate old handler shape; extra scratch copy/encode pass costs ~31%. |
| Max coil write from `&[bool]` | 295 ns | Bool-slice writes now pack into the byte-backed table. |
| Max coil write from packed wire bytes | 234 ns | Direct wire-byte writes merge packed bytes into the table. |
| Max coil packed bytes via `Vec<bool>` | 1.18 us | Approximate old handler shape; unpacking to bools and repacking is now clearly slower than the direct path. |
| Max coil read to packed wire bytes | 124 ns | Store slices packed table bytes directly into Modbus wire order. |
| Max coil read via bool buffer then pack | 1.37 us | Bool-slice reads now unpack from the packed table before the benchmark repacks to wire bytes. |
| Max FIFO read to BE wire bytes | 15.1 ns | Store writes the queue snapshot directly into the response payload buffer. |
| Max FIFO read via cloned `Vec<u16>` then pack | 27.9 ns | Approximate old handler shape; queue clone and second pack pass roughly double this microbench. |
| Max file-record read to BE wire bytes | 32.3 ns | Store writes a 122-register file sub-record directly into the response payload buffer. |
| Max file-record read via `u16` buffer then pack | 40.8 ns | Approximate old FC14 handler shape; extra scratch copy/encode pass costs ~26%. |
| Max file-record write from `&[u16]` | 12.3 ns | Slice baseline for existing store API. |
| Max file-record write from wire bytes | 11.2 ns | Direct FC15 path keeps borrowed request bytes through validation and avoids per-group allocation. |
| Max file-record wire bytes via `Vec<u16>` | 70.9 ns | Approximate old FC15 handler shape; per-group vector materialization dominates. |

The RTU-over-TCP CRC scan quick smoke was run with
`scripts/bench-local.sh codec rtu_tcp --quick --noplot` after changing the
frame-boundary scan to update CRC state incrementally:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| RTU/TCP FC 0x03 read request decode | 34.9 ns | Short-frame happy path remains tiny. |
| RTU/TCP max-size valid frame decode | 461 ns | Full-frame scan stays sub-microsecond. |
| RTU/TCP full corrupt buffer decode | 410 ns | No-match path now scans once instead of rehashing every prefix. |
| Old-style prefix rescan, full corrupt buffer | 42.4 us | Benchmark-only comparator for the previous scan strategy. |

The server handler quick smoke was run with
`scripts/bench-local.sh handler --quick --noplot` after adding direct
`process_request` baselines. These rows include request decode, protocol
validation, in-memory store access, and response construction, but exclude TCP,
TLS, RTU framing, and client-side work:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| FC01 max coil read | 123 ns | Byte-backed table lets the store emit packed response bytes directly. |
| FC02 max discrete-input read | 126 ns | Shares the same byte-backed packed response path as FC01. |
| FC03 max holding-register read | 29.2 ns | Direct BE register response path keeps full-size reads small. |
| FC0F max coil write | 255 ns | Packed request bytes merge directly into the byte-backed coil table. |
| FC10 max register write | 29.9 ns | Direct BE write path avoids the old request-payload `Vec<u16>`. |
| FC14 two-group file read | 48.4 ns | Direct final-buffer construction removes the previous response-data buffer and encode pass. |
| FC15 two-group file write | 57.3 ns | Stack-bounded validation staging removes the previous group `Vec` while preserving atomic framing validation. |
| FC17 max read/write registers | 42.7 ns | Read half now writes directly into the final response bytes. |
| FC18 FIFO two-value read | 27.0 ns | Direct FIFO response path is comparable to simple register handlers. |
| FC08 return query data | 21.9 ns | Direct diagnostic append path echoes borrowed request bytes into the response. |
| FC0C get comm event log | 17.9 ns | Direct event-log append path writes bounded event bytes into the response buffer. |
| FC11 report server ID | 22.2 ns | Direct server-id append path avoids cloning the store blob before response construction. |
| FC2B basic device identification | 31.1 ns | Stack-backed object selection removes the previous object/filter/selection vectors. |

The pipelined TCP Criterion quick smoke was run with
`scripts/bench-local.sh tcp-pipelined --quick --noplot`. This benchmark reports
read-holding-register throughput for repeated batches at each in-flight depth:

| In-flight | Quick-mode throughput |
|---:|---:|
| 1 | 48.1 Kelem/s |
| 2 | 85.0 Kelem/s |
| 4 | 136 Kelem/s |
| 8 | 199 Kelem/s |
| 16 | 250 Kelem/s |

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
