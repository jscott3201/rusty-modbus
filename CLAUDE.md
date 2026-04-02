# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build --workspace              # Build everything
cargo test --workspace               # Run all 537+ tests
cargo test -p rusty-modbus-codec     # Test a single crate
cargo test -p rusty-modbus-conformance -- spec_fc01  # Run specific conformance tests
cargo clippy --workspace --all-targets  # Lint (must be zero warnings; CI uses -Dwarnings)
cargo fmt --all --check              # Format check
cargo deny check                     # License/advisory checks
cargo check -p rusty-modbus --features full  # Verify facade with all features
```

Benchmarks:
```bash
cargo bench -p rusty-modbus-benchmarks --bench codec
cargo bench -p rusty-modbus-benchmarks --bench tcp_latency
cargo run -p rusty-modbus-benchmarks --bin stress-test -- --help
```

Python bindings (excluded from workspace — requires Python dev headers):
```bash
cd crates/rusty-modbus-python
python3 -m venv .venv && source .venv/bin/activate
pip install maturin pytest pytest-asyncio
maturin develop                      # Build + install into venv
pytest tests/ -v                     # Run Python tests (39 tests)
```

MSRV: **Rust 1.94**, Edition 2024, Resolver 3.

## Architecture

### Layered Crate Design

The workspace has 14 Rust crates + 1 Python binding crate, organized in dependency layers:

```
Layer 5: rusty-modbus-python (PyO3 bindings, excluded from workspace)
           ↓
Layer 4: CLI / Conformance / Sim / Facade
           ↓
Layer 3: client / server / gateway / pool
           ↓
Layer 2: tcp / rtu / tls (transport implementations)
           ↓
Layer 1: frame (Tokio codecs, CRC-16, owned Bytes types)
           ↓
Layer 0: codec (sans-IO, no_std) / types (newtypes, no_std)
```

**Foundation crates (`types`, `codec`) are `no_std` with zero allocator requirement.** They operate on `&[u8]` slices (decode) and `&mut [u8]` buffers (encode). This is deliberate — do not add `std` or `alloc` dependencies to them.

### Key Design Patterns

- **Sans-IO codec**: `rusty-modbus-codec` has no I/O or async dependencies. Encode/decode is pure data transformation. Transport-specific framing lives in `rusty-modbus-frame`.
- **Transport traits use RPITIT** (native `impl Future` in traits, no `async_trait` crate): `TransportSink::send()` and `TransportStream::recv()` in `rusty-modbus-tcp`. These traits are **not object-safe** due to RPITIT — use generics, not `Box<dyn>`.
- **Generic client**: `ModbusClient<S: TransportSink + Send + 'static = TcpSink>` supports any transport via the type parameter. `from_transport(sink, stream, config)` constructs from pre-connected halves (used for TLS). Default `TcpSink` keeps existing code backward-compatible.
- **Pipelined client**: 16-slot transaction ring in `TransactionManager`, background reader task, 500ms timeout sweep. Semaphore controls max in-flight requests.
- **Two-pool architecture** (per Modbus/TCP Guide §4.2.1): priority pool (configured devices, never evicted) + non-priority pool (LRU eviction).
- **`OwnedResponsePdu`**: wraps `Bytes` for zero-copy response sharing through the transaction manager pipeline.
- **`#![forbid(unsafe_code)]`** on all Rust crates (not the Python crate — PyO3 macros generate unsafe internally).

### Python Bindings (`crates/rusty-modbus-python/`)

PyO3 0.28 + maturin crate providing `rusty_modbus` Python package. Excluded from workspace via `Cargo.toml` `exclude` (requires Python headers to build cdylib).

- **`ModbusClient`** — async client, methods return awaitables via `pyo3_async_runtimes::tokio::future_into_py`
- **`SyncModbusClient`** — blocking wrapper owning its own `tokio::runtime::Runtime`
- **`InnerClient` enum** — dispatches to `ModbusClient<TcpSink>` or `ModbusClient<TlsSink>` (duplicated in both client modules for isolation)
- **Error mapping** — `client_error_to_pyerr()` free function (not `From` impl — orphan rule). All exceptions subclass `ModbusError`.
- **Type stubs** — `rusty_modbus.pyi` + `py.typed` at crate root, shipped by maturin automatically

### Conformance Test Suite

`rusty-modbus-conformance/tests/` contains 36 test files organized by spec section. Tests are named `spec_fc01_*`, `spec_fc02_*`, etc. for function codes, plus `spec_header`, `spec_exceptions`, `spec_broadcast`, `spec_rtu_framing`, `spec_tls_handshake` for protocol-level validation. Tests use localhost loopback with per-test mock servers.

### Facade Crate Feature Flags

`rusty-modbus` re-exports subcrates behind feature flags. `tcp` is the only default. `full` enables everything. When adding a new subcrate, it must be wired into the facade's `Cargo.toml` features and conditionally re-exported.

## CI Pipeline

GitHub Actions (`.github/workflows/ci.yml`):
- **Tier 1** (every push/PR): fmt, clippy (`--locked`), test (`--locked`, Linux + macOS + Windows), cargo-deny
- **Tier 2** (v* tags): validate version → publish crates sequentially → build CLI binaries for 5 targets → GitHub release

All CI commands use `--locked` to enforce `Cargo.lock` and prevent MSRV-breaking dependency resolution.

Crate publish order matters due to inter-crate dependencies: types → codec → frame → tcp → rtu → tls → pool → client → server → gateway → sim → facade.

## Conventions

- All new Rust crates must have `#![forbid(unsafe_code)]` (exception: Python crate)
- Zero clippy warnings enforced (`RUSTFLAGS: -Dwarnings` in CI)
- Async tests use `#[tokio::test]`, not a custom runtime
- Transport implementations must impl both `TransportSink` and `TransportStream` traits
- Server backends impl the `DataStore` trait (8 async methods covering 4 Modbus data tables)
- `deny.toml` ignores: RUSTSEC-2025-0134 (rustls-pemfile, awaiting upstream), RUSTSEC-2026-0009 (time crate DoS, transitive/low-risk)
