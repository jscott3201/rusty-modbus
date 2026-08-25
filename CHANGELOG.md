# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added synchronous, idempotent `ModbusClient::abort()` for immediate
  cancellation without a live Tokio runtime. The Python async and sync clients
  expose the same method, and Python `ClientConfig` now exposes the Rust client
  shutdown timeout as `shutdown_timeout_secs`.
- Added immutable server metrics and `ShutdownOutcome::{Drained, Forced}`.
  Python exposes the same counters through a read-only `ServerMetrics` class and
  returns a typed `"drained" | "forced"` result from `ModbusServer.stop()`.
- Added `modbus server --shutdown-timeout-secs`; the CLI reports the shutdown
  outcome and connection, request, admission-rejection, and accept-error counters.
- Added defaulted atomic FC 0x16/0x17 `DataStore` hooks and matching optional
  Python store callbacks. Existing custom stores still compile and return Illegal
  Function for these operations until they implement the new capability.
- Added a bounded Rust `DataStore::handle_custom_function` hook for non-standard
  function codes. Existing stores keep the Illegal Function default; the hook is
  not exposed through Python, the CLI, or the simulator.
- Added the `rusty-modbus-sim <CONFIG>` executable. It reports the actual bound
  address and Unit Identifier on stdout, serves until SIGINT/SIGTERM on Unix or
  Ctrl-C elsewhere, and waits for the existing bounded server stop path.

### Changed

- Added `ClientError::UnexpectedResponseUnitId { expected, got }`. Downstream
  Rust code that exhaustively matches `ClientError` must handle this variant.
- Added `ClientError::UnexpectedResponseLength`,
  `ClientError::UnexpectedResponsePadding`, and
  `DecodeError::InvalidRegisterDataLength` /
  `EncodeError::InvalidRegisterDataLength`. Exhaustive Rust matches on these
  error enums must handle the new variants.
- Simulator YAML now rejects unknown or duplicate fields, unsupported update
  modes and faults, invalid direct TCP Unit Identifiers and listen addresses,
  noncanonical static bounds, zero or overflowing blocks, excess initial
  values, and same-table overlaps before runtime state is created.

### Fixed

- Typed writes are no longer replayed after ambiguous response or transport
  timeouts. Replay-safe reads retain bounded timeout retries, and configured
  Server Device Busy (`0x06`) responses remain retryable for reads and writes.
- Acknowledge (`0x05`) is now a terminal typed exception even when manually
  included in `retryable_exceptions`; it no longer triggers automatic replay.
- Client attempt timeouts now use the nearest deadline in the fixed 16-slot
  transaction ring instead of a periodic 500 ms sweep. One logical request holds
  its admission permit across its bounded attempts and backoff. Its operation
  envelope starts after admission; waiting for a permit is not timed.
- Client responses now successfully complete a Modbus/TCP request only when the
  transaction ID, Unit Identifier, and normal or exception function identity
  match. RTU clients ignore responses for other units while retaining the active
  request.
- Typed FC01/02 reads now reject excess data and nonzero final-byte padding.
  Typed FC03/04/17 reads reject excess register data instead of truncating it,
  and their decoders reject odd register byte counts. Raw FC03 responses retain
  their existing `Bytes` storage after validation rather than copying it.
- Client shutdown now seals admission before drain, keeps response and deadline
  processing active for admitted operations, and enforces one absolute shutdown
  deadline for admitted sink waits, sends, response waits, retries, backoff, and
  broadcasts. Admission waiters are rejected when the seal closes the
  semaphore. Remaining work receives `ClientError::ShuttingDown`, and shutdown
  joins the reader and deadline tasks before returning. Final-owner `Drop` uses
  the non-waiting abort path.
- Server configuration now rejects zero connection, transaction, and shutdown
  limits before bind. Server shutdown drops the listener before drain, lets an
  admitted sequential request finish, rejects the next frame, and aborts and
  joins remaining connection tasks at one absolute deadline. Concurrent stop
  callers share that deadline and outcome. Listener admission is atomic and
  reports saturation, access-control rejection, and setup cleanup accurately.
  `max_transactions` remains configuration-only pending per-connection
  pipelining and runtime enforcement.
- FC 0x16 and FC 0x17 now execute through one atomic store callback. The built-in
  `InMemoryStore` holds one write guard across each compound operation, including
  the FC 0x17 post-write read.

## [0.1.1]

### Added

- Added SHA-bound correctness and benchmark recording, a structurally validated
  conformance ledger, fixed-seed parser resilience properties, and pinned
  retained fuzz replay.
- Added a strict physical RTU configuration profile for 8E1, 8O1, and 8N2,
  including direction-aware Unit Identifier validation and ceiling-based timing.
- Added a timestamp-driven, fixed-buffer RTU frame assembler with bounded
  recovery diagnostics and retained fuzz coverage.

### Changed

- Refreshed the local and Docker benchmark report and corrected the Python
  cargo-deny command ordering.

### Security

- Updated PyO3 and pyo3-async-runtimes to 0.29 to address
  RUSTSEC-2026-0176 and RUSTSEC-2026-0177 in the Python binding lockfile.
- Updated crossbeam-epoch to 0.9.20, the first patched release for
  RUSTSEC-2026-0204.

### Boundaries

- The timestamp-driven assembler is not integrated with the physical
  `SerialTransport`. Its repository tests and fuzz targets do not establish
  OS/USB read-chunk invariance or physical interoperability.
- The recorded evidence is repository-scoped. This candidate makes no claim of
  independent interoperability, formal certification, or 1.0 readiness.

## [0.1.0] - 2026-06-03

The first public release. `rusty-modbus` is a layered, async Modbus protocol
stack for Rust.

### Added

- **Foundation** — `no_std` `types` (newtypes) and sans-IO `codec`
  (encode/decode on `&[u8]`/`&mut [u8]`), plus a `frame` layer with Tokio codecs
  and CRC-16.
- **Transports** — Modbus/TCP, RTU (serial and RTU-over-TCP), and Modbus/TCP
  Security (TLS 1.3 mutual auth). Transport traits use native RPITIT.
- **Pipelined client** — generic `ModbusClient<S>` over any transport, with a
  16-slot transaction ring, background reader, and timeout sweep. 14 typed
  function codes.
- **Server** — `DataStore`-backed async server dispatching all 19 standard
  function codes. The built-in `InMemoryStore` serves register/coil access,
  Read Device Identification (MEI), File Record (0x14/0x15), FIFO Queue (0x18),
  Read Exception Status (0x07), Diagnostics loopback (0x08), and Report Server
  ID (0x11); Get Comm Event Counter/Log (0x0B/0x0C) are exposed as `DataStore`
  hooks with conformant defaults. The new capability methods are default-bodied,
  so existing `DataStore` implementations keep compiling unchanged.
- **Gateway** — Modbus/TCP ↔ RTU translation.
- **Connection pool** — two-pool model (priority devices + LRU non-priority)
  per the Modbus/TCP Implementation Guide §4.2.1.
- **Python bindings** — `rusty_modbus` (PyO3) with async and sync clients.
- **Conformance suite** — spec-grounded tests against Modbus V1.1b3 / TCP /
  Serial / Security specifications.

### Fixed (pre-release hardening)

- **Client**: the background reader no longer dies on a benign idle read
  timeout; bit/register reads guard short responses instead of panicking or
  silently truncating; responses are verified to echo the requested function
  code (V1.1b3 §4.4).
- **RTU client**: requests over the pipelined client now correlate correctly —
  RTU is forced single-in-flight and responses are matched to the outstanding
  request (previously every RTU request timed out).
- **Gateway**: relayed responses are validated to come from the addressed unit
  with the expected function code before forwarding.
- **Server**: an unrecognized Diagnostics (0x08) sub-function now returns
  IllegalFunction (0x01) rather than IllegalDataValue (0x03), per V1.1b3 §6.8
  (Figure 18).
- **Pool**: genuine separate budgets for the priority and non-priority pools so
  idle priority connections can no longer starve non-priority requests; wired
  the per-device connection cap and exponential reconnect backoff; pre-connect
  tasks are aborted on shutdown.
- **TLS**: optional hostname/SNI verification; client role extraction from the
  peer certificate (with a robust ASN.1 parser) surfaced from `accept()`; an
  `authorize()` helper; the missing-client-cert warning now fires in release
  builds.

### Security

- TLS enforces TLS 1.3 with mutual x.509v3 authentication by default.
- `#![forbid(unsafe_code)]` on all Rust crates (except the PyO3 bindings, where
  the macros generate `unsafe` internally).

[Unreleased]: https://github.com/jscott3201/rusty-modbus/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/jscott3201/rusty-modbus/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jscott3201/rusty-modbus/releases/tag/v0.1.0
