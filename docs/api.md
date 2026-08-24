# API Surfaces

This document summarizes the public Rust and Python surfaces for the 0.1.1
candidate. The project remains pre-1.0, and these APIs may change.

## Conformance evidence

The canonical ledger records profile-scoped dispositions and evidence for the
[TCP client](conformance/ledger.md#profile-tcp-client),
[TCP server](conformance/ledger.md#profile-tcp-server),
[physical RTU client](conformance/ledger.md#profile-physical-rtu-client),
[physical RTU responder](conformance/ledger.md#profile-physical-rtu-responder),
[gateway](conformance/ledger.md#profile-gateway),
[Modbus/TCP Security](conformance/ledger.md#profile-modbus-security),
[simulator](conformance/ledger.md#profile-simulator), and
[RTU-over-TCP extension](conformance/ledger.md#profile-rtu-over-tcp-extension).
The current positive profile claims are seeded at `implemented`. The ledger
defines the higher evidence levels and their requirements. The physical
RTU responder is `not-implemented`. Each profile lists its compatibility
deviations and evidence gaps.

## Rust Crates

The Rust workspace uses edition 2024 with MSRV 1.95. The root facade crate is
`rusty-modbus`; lower-level crates are also publishable so users can depend on a
smaller API when they only need codec, transport, server, or simulator pieces.

| Crate | Public role |
|---|---|
| `rusty-modbus` | Facade crate with feature-gated re-exports. Default feature is `tcp`. |
| `rusty-modbus-types` | `no_std` Modbus newtypes, constants, enums, and fixed wire types. |
| `rusty-modbus-codec` | Sans-IO PDU request/response encode and decode. |
| `rusty-modbus-frame` | MBAP/RTU framing, CRC-16, Tokio codecs, and owned `Bytes` response types. |
| `rusty-modbus-tcp` | TCP transport traits and Modbus/TCP transport implementation. |
| `rusty-modbus-rtu` | Serial RTU and RTU-over-TCP transports, plus timestamp-driven RTU assembly. |
| `rusty-modbus-tls` | Modbus/TCP Security TLS transport and role primitives using rustls; not a composed secured server. |
| `rusty-modbus-client` | Pipelined async client with typed function-code methods. |
| `rusty-modbus-server` | Async server and pluggable `DataStore` trait. |
| `rusty-modbus-pool` | Connection pooling for client workloads. |
| `rusty-modbus-gateway` | TCP frontend with RTU-over-TCP backend routing and frame translation; not a physical serial gateway. |
| `rusty-modbus-sim` | YAML-driven in-process simulator. |

The CLI crate is intentionally `publish = false`; release binaries are produced
by the GitHub release pipeline instead of crates.io.

## Facade Features

| Feature | Default | API exposed |
|---|---:|---|
| `tcp` | yes | `rusty_modbus::tcp`, `rusty_modbus::client`, `rusty_modbus::Client` |
| `rtu` | no | `rusty_modbus::rtu` configuration and RTU-over-TCP support, without physical serial dependencies |
| `rtu-serial` | no | Physical serial support in `rusty_modbus::rtu`, in addition to `rtu` |
| `rtu-tcp` | no | Alias for `rtu`, without physical serial dependencies |
| `tls` | no | `rusty_modbus::tls` |
| `server` | no | `rusty_modbus::server`, `rusty_modbus::Server` |
| `gateway` | no | `rusty_modbus::gateway`, `rusty_modbus::Gateway` |
| `pool` | no | `rusty_modbus::pool` |
| `full` | no | All optional features |

The foundation crates `types`, `codec`, and `frame` are always re-exported by
the facade crate.

## Physical RTU configuration

`rusty_modbus_rtu::RtuConfig` and `SerialTransport::open` are the compatibility
path. Their public fields, 9600/8N1 default, and legacy timing calculations are
unchanged. Code that requires a Modbus serial character format uses
`StrictRtuConfig` with `SerialTransport::open_strict` instead. The strict type
accepts 8E1, 8O1, and 8N2 and cannot contain a zero baud rate.

`StrictRtuConfig::resolve` returns a `ResolvedRtuConfig` with the concrete data,
parity, and stop-bit settings; response timeout; character time; t1.5; t3.5;
and timing mode. Character-calculated values use independent integer
nanosecond ceiling calculations through 19,200 bit/s. Higher rates use the
recommended fixed 750 microsecond t1.5 and 1.750 millisecond t3.5 values. The
strict serial halves expose the same resolved snapshot that supplied the port
settings and transmit delay.

Strict physical sends accept Unit Identifiers 0 through 247 as destinations,
preserving address zero for broadcast. Strict receives accept responder sources
1 through 247. This is address-class validation only: expected-peer correlation,
broadcast operation policy, and physical RTU responder support are not part of
this API.

## RTU frame assembler

`rusty_modbus_rtu::RtuFrameAssembler` accepts explicit byte timestamps and
tokenized t3.5 deadline events. `RtuTiming` validates `0 < t1.5 < t3.5` and can
be constructed from `ResolvedRtuConfig`. The assembler keeps one fixed
`MAX_RTU_ADU_SIZE` candidate, enters quarantine after an inter-character gap or
overlength input, and returns `OwnedRtuAdu` only when a t3.5 boundary closes a
4-through-256-byte candidate with a valid whole-buffer CRC. Diagnostic counters
are fixed and saturating.

Callers are responsible for monotonic per-byte timestamps that preserve wire
timing. This API is not wired to `SerialTransport` or `AsyncRead`; it cannot
derive timing concealed within one OS/USB read. The assembler tests and fuzz
target therefore do not prove physical interoperability or read-chunk
invariance. PDU function semantics remain the codec's responsibility.

## Rust Client

`rusty_modbus_client::ModbusClient` is async and transport-generic. The default
constructor connects over Modbus/TCP:

```rust
use rusty_modbus::client::{ClientConfig, ModbusClient};
use rusty_modbus::types::UnitId;

let client = ModbusClient::connect("127.0.0.1:502".parse()?, ClientConfig::default()).await?;
let values = client.read_holding_registers(UnitId(1), 0, 10).await?;
client.shutdown().await;
```

The client supports typed methods for the public client function-code surface:

- Coils and discrete inputs: FC 0x01, 0x02, 0x05, 0x0F.
- Registers: FC 0x03, 0x04, 0x06, 0x10, 0x16, 0x17.
- File records and FIFO: FC 0x14, 0x15, 0x18.
- Device identification: FC 0x2B / MEI 0x0E.

Modbus/TCP supports up to 16 concurrent in-flight transactions. RTU transports
force one in-flight request because RTU frames have no transaction ID.
`ClientConfig::timeout` applies to each attempt after semaphore admission.
Waiting for admission is not timed; the bounded logical-request envelope starts
when a permit is acquired. Response and transport timeouts are retried only for
replay-safe reads; typed writes are not replayed because a timeout or send
failure does not prove non-execution. A configured Server Device Busy (`0x06`)
response can retry either request kind. Acknowledge (`0x05`) is returned as
`ClientError::Exception` to report accepted, still-processing work; the
application owns any completion check.

`shutdown().await` atomically seals request admission, lets admitted operations
drain with response and deadline processing still active, and cancels remaining
work at `ClientConfig::shutdown_timeout`. It joins the reader and deadline tasks
before returning. Concurrent calls share one shutdown coordinator. `abort()` is
the synchronous alternative: it seals and cancels immediately without waiting
or requiring a live Tokio runtime. A later `shutdown().await` joins the tasks.
Dropping the final client owner follows the abort path; dropping a non-final
`Arc` handle does not stop shared work.

These lifecycle operations do not guarantee a flush or physical close while the
generic sink remains owned. `TransportSink` has no close method, and cancellation
may happen after some or all request bytes were written. Device Identification
admits one page at a time, so a shutdown between pages rejects the next page.

These surfaces map to the [TCP client](conformance/ledger.md#profile-tcp-client),
[physical RTU client](conformance/ledger.md#profile-physical-rtu-client),
[Modbus/TCP Security](conformance/ledger.md#profile-modbus-security), and
[RTU-over-TCP extension](conformance/ledger.md#profile-rtu-over-tcp-extension)
profiles.

## Rust Server

`rusty_modbus_server::ModbusServer` serves Modbus/TCP with a pluggable
`DataStore`. The required `DataStore` methods cover the four standard data
tables:

- Coils.
- Discrete inputs.
- Holding registers.
- Input registers.

Optional `DataStore` methods cover file records, FIFO queues, Report Server ID,
Diagnostics, Read Exception Status, Get Comm Event Counter, and Get Comm Event
Log. Defaults return `IllegalFunction` for unsupported optional operations, so
stores only override what they support. The server crate maps to the
[TCP server profile](conformance/ledger.md#profile-tcp-server); there is no
first-party [physical RTU responder](conformance/ledger.md#profile-physical-rtu-responder),
and TLS primitives do not compose a secured server on their own.

`ServerConfig::validate` runs before bind and rejects zero
`max_connections`, `max_transactions`, or `shutdown_timeout`. Values of
`max_transactions` above 16 remain valid configuration, but the server does not
enforce that field at runtime: each TCP connection processes requests
sequentially.

`ModbusServer::stop().await` seals listener and request admission and records
one absolute deadline. Idle connections are signalled to exit and are joined. A
request admitted before the seal may finish its handler and response send; a
later frame on that connection is rejected. The supervisor drops the listener
before waiting, returns `ShutdownOutcome::Drained` when all connection tasks
finish, or aborts and joins the remainder before returning
`ShutdownOutcome::Forced`. Concurrent and repeated calls receive the same result
even if an earlier stop future was cancelled.

`ModbusServer::metrics()` returns an immutable `ServerMetrics` snapshot with
active connections and requests, accepted connections, access-control denials,
connection-limit rejections, and accept errors. Accept failures use
shutdown-interruptible exponential backoff from 10 milliseconds to 1 second.

Tokio abort is cooperative. A datastore future or Python callback that does not
yield can delay task termination beyond the configured deadline. Dropping a
server requests a synchronous, non-waiting supervisor abort; it does not provide
graceful completion or an immediate-rebind guarantee.

The built-in `InMemoryStore` is thread-safe and optimized for common paths:

- Coil and discrete-input tables are stored as packed byte-backed bit tables.
- Store hooks can write packed coils, register bytes, file records, FIFO data,
  diagnostics, and server-identification payloads directly into response
  buffers.
- File-record writes are validated before commit to preserve all-or-nothing
  behavior.

## Codec And Framing

The codec is written as a Sans-IO layer over caller-owned byte slices. Decode
paths validate function-specific envelopes and borrow variable-length payloads
from the input buffer. Owned frame responses use `bytes::Bytes` slicing to keep
payload ownership cheap without copying full response bodies.

The current capability and evidence inventory includes validation for:

- FC 0x14/0x15 file-record reference types and byte counts.
- FC 0x18 FIFO response limits.
- FC 0x2B / MEI 0x0E request/response control fields and pagination behavior.
- PDU length, MBAP framing, RTU CRC, and exception response handling.

## Python Package

The Python bindings live in `crates/rusty-modbus-python` and are excluded from
the Cargo workspace because they build a CPython extension module. The package
name is `rusty_modbus`, requires Python 3.14 or newer, and is validated against
standard CPython 3.14 plus free-threaded 3.14t.

Public Python classes:

| Class | Role |
|---|---|
| `ClientConfig`, `RetryConfig`, `TlsConfig` | Read-only connection configuration objects. |
| `ModbusClient` | Asyncio client. Methods return `Awaitable[...]`. |
| `SyncModbusClient` | Blocking client with the same typed operation surface. |
| `ServerConfig`, `StoreConfig` | Read-only server and store sizing configuration. |
| `ServerMetrics` | Read-only server connection, request, rejection, and accept-error snapshot. |
| `InMemoryStore` | Python-visible in-memory store for local servers. |
| `ModbusServer` | Background Modbus/TCP server wrapper. |
| `DeviceIdentification` | Result object for FC 0x2B / MEI 0x0E reads. |

The Python client exposes coils, registers, mask write, read/write multiple
registers, FIFO, file records, and device identification operations. This list
is not a parity claim; unresolved surface differences are recorded under
[CONF-008](conformance/ledger.md#requirement-conf-008).
Both client classes expose synchronous `abort()` methods. Their context-manager
exits still call graceful `shutdown`; `ClientConfig.shutdown_timeout_secs`
controls that drain and defaults to 10 seconds.

Python `ModbusServer.stop()` blocks without holding the GIL and returns the
literal string `"drained"` or `"forced"`. `ModbusServer.metrics()` returns a
read-only `ServerMetrics` object with the same fields as Rust. Server
`shutdown_timeout_secs` must be finite and positive. Synchronous Python
callbacks are not preempted while they are executing; a callback that does not
return can delay forced shutdown beyond the deadline.

The gateway and simulator are tracked separately under the
[gateway](conformance/ledger.md#profile-gateway) and
[simulator](conformance/ledger.md#profile-simulator) profiles. Simulator fields
that are parsed but do not affect runtime behavior remain explicit evidence
gaps rather than implied capabilities.

## Python Server Store Protocols

Python-backed stores are structural protocols in `rusty_modbus.pyi`. They are
typing-only contracts, not runtime classes exported by the extension module; use
them under `typing.TYPE_CHECKING` or in annotations evaluated by a type checker.

`DataStore` requires the four core data-table callbacks:

- `read_coils`, `write_coil`, `write_coils`.
- `read_discrete_inputs`.
- `read_holding_registers`, `write_register`, `write_registers`.
- `read_input_registers`.

Optional protocol extensions document additional callbacks:

| Protocol | Optional callbacks |
|---|---|
| `FileRecordDataStore` | `read_file_record`, `write_file_record` |
| `FifoDataStore` | `read_fifo_queue` |
| `SerialDiagnosticsDataStore` | `read_exception_status`, `get_comm_event_counter`, `get_comm_event_log`, `diagnostic` |
| `ServerIdentificationDataStore` | `report_server_id` |

Callbacks may raise the Python Modbus exception classes to control the wire
exception code. Unknown Python exceptions map to Server Device Failure.

## Python Typing Guarantees

The package ships `py.typed` plus `rusty_modbus.pyi`. The local and CI typing
gates are:

- `mypy.stubtest rusty_modbus`.
- `pyright --verifytypes rusty_modbus`.
- `pyright --project typing_tests/pyrightconfig.json`.

The public-contract pyright project verifies:

- Async methods return `Awaitable[...]`.
- Sync methods return concrete values.
- Config and result objects expose read-only properties.
- Byte-like file-record and diagnostics inputs are accepted in type signatures.
- Python store objects satisfy the datastore protocols structurally.

## Release Model

Everyday work lands on `dev`. Release work is a PR from `dev` into `main`; that
PR runs the broader `release.yml` gate across Linux, macOS, and Windows, plus
feature checks, cargo-deny, cargo-audit, and the Python binding compile check.

A `v*` tag on `main` triggers `publish.yml`. The tag must match the workspace
version exactly. The publish workflow then:

1. Validates tag version against `Cargo.toml`.
2. Runs the inter-crate publish-version guard.
3. Publishes crates to crates.io in dependency order.
4. Builds Python source and Linux manylinux wheels for CPython 3.14 and 3.14t.
5. Publishes Python distributions to PyPI through Trusted Publishing.
6. Builds CLI binaries for GitHub Releases.

PyPI Trusted Publishing should be configured for repository
`jscott3201/rusty-modbus`, workflow `.github/workflows/publish.yml`, and
environment `release`.

Do not bump versions as part of ordinary docs, CI, spec, performance, or typing
PRs. Version changes are release decisions.
