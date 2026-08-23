# Rusty Modbus

A Modbus protocol workspace written in Rust, with Python bindings for selected
client and server workflows. It includes TCP clients and servers, a physical RTU
client, a separately labeled RTU-over-TCP extension, TLS transport primitives,
TCP-to-RTU-over-TCP gatewaying, connection pooling, simulation, packaging, benchmarks,
and a diagnostic CLI.

[![CI](https://github.com/jscott3201/rusty-modbus/actions/workflows/ci.yml/badge.svg)](https://github.com/jscott3201/rusty-modbus/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Conformance scope

Public capability claims are profile-scoped: [TCP client](docs/conformance/ledger.md#profile-tcp-client),
[TCP server](docs/conformance/ledger.md#profile-tcp-server),
[physical RTU client](docs/conformance/ledger.md#profile-physical-rtu-client),
[physical RTU responder](docs/conformance/ledger.md#profile-physical-rtu-responder),
[gateway](docs/conformance/ledger.md#profile-gateway), [Modbus/TCP Security](docs/conformance/ledger.md#profile-modbus-security),
[simulator](docs/conformance/ledger.md#profile-simulator), and the
[RTU-over-TCP extension](docs/conformance/ledger.md#profile-rtu-over-tcp-extension).
The generated ledger separates implementation, internal verification,
interoperability, and formal certification. Repository tests and private tool
use do not imply Modbus Organization testing or certification.

## Features

- **Transport components** — Modbus/TCP, a physical serial RTU client, an RTU-over-TCP extension, and TLS 1.3 primitives with mutual authentication enabled by default
- **Pipelined async client** — 16-slot transaction ring with background reader task
- **Pluggable server** — async `DataStore` trait for custom register backends
- **TCP-to-RTU-over-TCP gateway** — request routing and frame translation; no physical serial gateway
- **Connection pooling** — two-pool architecture with idle eviction and reconnect backoff
- **YAML simulator** — initial register maps and device profiles; parsed update and fault settings are not active
- **CLI tool** — read/write/server/shell/dashboard/discover commands with JSON output
- **Python bindings** — CPython 3.14/3.14t wheels with typed async/sync clients and server stores
- **Conformance evidence** — 70 requirement rows and the live Rust conformance-test inventory in the [profile ledger](docs/conformance/ledger.md)
- **`no_std` foundation** — types and codec crates work without allocator
- **Unsafe-code policy** — workspace library crates use `#![forbid(unsafe_code)]`
- **Zero clippy warnings**, fast `dev` CI and broader release-gate CI

## Rust Quick Start

```toml
[dependencies]
rusty-modbus = "0.1"
tokio = { version = "1", features = ["full"] }
```

```rust
use rusty_modbus::client::{ModbusClient, ClientConfig};
use rusty_modbus::types::UnitId;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ClientConfig {
        timeout: Duration::from_secs(5),
        ..ClientConfig::default()
    };

    let client = ModbusClient::connect("127.0.0.1:502".parse()?, config).await?;

    // Read 10 holding registers starting at address 0
    let registers = client.read_holding_registers(UnitId(1), 0, 10).await?;

    for (i, &val) in registers.iter().enumerate() {
        println!("Register {i:>3}: {val:#06X} ({val})");
    }

    client.shutdown().await;
    Ok(())
}
```

The `rusty-modbus` facade crate enables the `tcp` client feature by default.
Enable `server`, `rtu`, `tls`, `pool`, `gateway`, or `full` when those modules
are needed.

## Python Quick Start

The Python package is built from `crates/rusty-modbus-python` with PyO3 and
maturin. It requires Python 3.14 or newer and is validated against both standard
CPython 3.14 and free-threaded 3.14t.

```bash
scripts/ci-python.sh
```

That command builds a fresh wheel, installs it into an isolated uv environment,
runs pytest, runs stubtest, and runs pyright type checks.

```python
import asyncio

import rusty_modbus


async def main() -> None:
    client = await rusty_modbus.ModbusClient.connect("127.0.0.1:502")
    try:
        registers = await client.read_holding_registers(1, 0, 10)
        print(registers)
    finally:
        await client.shutdown()


asyncio.run(main())
```

For non-async scripts, use `rusty_modbus.SyncModbusClient`. For server tests or
simulators, use `rusty_modbus.ModbusServer.start()` with either
`rusty_modbus.InMemoryStore` or a Python object matching the `DataStore`
protocols described in [docs/api.md](docs/api.md).

## CLI Tool

```bash
# From a source checkout
cargo run -p rusty-modbus-cli -- --help

# After a tagged release, download the prebuilt modbus binary from GitHub Releases.
modbus --help

# Read holding registers
modbus read -H 192.168.1.100 holding 0 10

# Write a single register
modbus write -H 192.168.1.100 register 0 0x1234

# Interactive shell
modbus shell -H 192.168.1.100

# Discover devices on a subnet
modbus discover --range 192.168.1.0/24

# Run an in-memory Modbus/TCP server
modbus server --listen 0.0.0.0:5502 --holding 0=0x1234

# JSON output (for scripting)
modbus read -H 192.168.1.100 holding 0 10 --format json

# Structured diagnostics stay on stderr; command output stays on stdout
modbus --log-filter rusty_modbus_client=debug --log-format json \
  read -H 192.168.1.100 holding 0 10 --format json

# Write diagnostics to a file instead of stderr
modbus --log-filter info --log-file modbus.log discover --range 192.168.1.0/24
```

## Docker

The Docker image packages the `modbus` CLI. By default it runs an in-memory
server on port 5502 as a non-root user; override the command to run client,
shell, dashboard, or discovery modes.

```bash
# Build the Alpine runtime image
scripts/docker-build.sh

# Build the distroless runtime image
RUSTY_MODBUS_DOCKER_TARGET=distroless \
RUSTY_MODBUS_DOCKER_TAG=rusty-modbus:distroless \
  scripts/docker-build.sh

# Run the default server
docker run --rm -p 5502:5502 rusty-modbus:local

# Run client commands from the same image
docker run --rm rusty-modbus:local \
  --host host.docker.internal --port 5502 --unit-id 1 read hr 0 1

# Run the interactive shell
docker run --rm -it rusty-modbus:local \
  --host host.docker.internal --port 5502 --unit-id 1 shell

# Docker-only e2e smoke: server container + client containers
scripts/docker-smoke.sh

# Build and run the Alpine benchmark target
scripts/docker-bench.sh --duration 5 --clients 1 --in-flight 8 --json

# Build and run the distroless benchmark target
RUSTY_MODBUS_DOCKER_TARGET=benchmark-distroless \
  scripts/docker-bench.sh --duration 5 --clients 1 --in-flight 8 --json

# Full local Docker check: Alpine smoke, distroless smoke, benchmark smoke
scripts/docker-ci.sh

# Comparable local + Docker benchmark matrix
scripts/bench-suite.sh all
```

## Workspace Structure

```
crates/
  rusty-modbus-types/       Enums, newtypes, constants (no_std, zerocopy)
  rusty-modbus-codec/       Sans-IO encode/decode (no_std)
  rusty-modbus-frame/       Tokio codecs, CRC-16, owned Bytes types
  rusty-modbus-tcp/         Transport traits + TCP implementation
  rusty-modbus-rtu/         Serial + RTU-over-TCP transport
  rusty-modbus-tls/         TLS 1.3 transport (rustls + ring)
  rusty-modbus-pool/        Two-pool connection pooling
  rusty-modbus-client/      Pipelined async client
  rusty-modbus-server/      Pluggable DataStore server
  rusty-modbus-gateway/     TCP <-> RTU-over-TCP bridge
  rusty-modbus-sim/         YAML simulator + device profiles
  rusty-modbus-cli/         CLI binary (read/write/server/shell/dashboard/discover)
  rusty-modbus/             Facade crate with feature flags
  rusty-modbus-conformance/ Requirement-oriented test suite
  rusty-modbus-python/      PyO3 bindings, excluded from the Rust workspace
benchmarks/                 Criterion benchmarks + stress-test binary
```

## Feature Flags

The `rusty-modbus` facade crate re-exports subcrates behind feature flags:

| Feature   | Default | Pulls In |
|-----------|---------|----------|
| `tcp`     | yes     | `rusty-modbus-tcp`, `rusty-modbus-client` |
| `rtu`     | no      | `rusty-modbus-rtu` |
| `rtu-tcp` | no      | alias for `rtu` |
| `tls`     | no      | `rusty-modbus-tls` (rustls + ring) |
| `server`  | no      | `rusty-modbus-server` |
| `gateway` | no      | `rusty-modbus-gateway` + `rtu` |
| `pool`    | no      | `rusty-modbus-pool` |
| `full`    | no      | all of the above |

## Function-code capability inventory

The client exposes **14** typed function codes. The server dispatches all 19
standard codes: the built-in `InMemoryStore` serves **17**, and the `DataStore`
trait exposes hooks for the remaining two.

| Function Code | Name | Client | Server |
|---------------|------|--------|--------|
| 0x01 | Read Coils | yes | yes |
| 0x02 | Read Discrete Inputs | yes | yes |
| 0x03 | Read Holding Registers | yes | yes |
| 0x04 | Read Input Registers | yes | yes |
| 0x05 | Write Single Coil | yes | yes |
| 0x06 | Write Single Register | yes | yes |
| 0x07 | Read Exception Status | no | yes |
| 0x08 | Diagnostics | no | yes |
| 0x0B | Get Comm Event Counter | no | hook † |
| 0x0C | Get Comm Event Log | no | hook † |
| 0x0F | Write Multiple Coils | yes | yes |
| 0x10 | Write Multiple Registers | yes | yes |
| 0x11 | Report Server ID | no | yes |
| 0x14 | Read File Record | yes | yes |
| 0x15 | Write File Record | yes | yes |
| 0x16 | Mask Write Register | yes | yes |
| 0x17 | Read/Write Multiple Registers | yes | yes |
| 0x18 | Read FIFO Queue | yes | yes |
| 0x2B/0x0E | Read Device Identification (MEI) | yes | yes |

† Dispatched to a `DataStore` method; the built-in `InMemoryStore` returns
`IllegalFunction`. Implement the trait method to serve device-specific counters.

The serial-line diagnostics codes (0x07/0x08/0x0B/0x0C/0x11) are accepted over
Modbus/TCP through `DataStore` methods. Their defaults return `IllegalFunction`;
override them for device-specific behavior. This table is an API inventory, not
a blanket conformance claim.

## API Documentation

- [docs/api.md](docs/api.md) summarizes the current Rust and Python public API
  surfaces, including feature flags, server stores, typing contracts, and the
  release branch model.
- Rust crates are documented with rustdoc and are configured for docs.rs with
  all facade features enabled once the 0.1.0 crates are published.
- Python typing is shipped through `py.typed` and `rusty_modbus.pyi`, with
  `stubtest`, `pyright --verifytypes`, and a strict pyright public-contract test
  in CI.

## Development

```bash
# Build entire workspace
cargo build --workspace

# Run the workspace test gate with nextest + doctests
scripts/test-local.sh

# Run the Python binding wheel, pytest, stubtest, and pyright gate
scripts/ci-python.sh

# Lint (must be zero warnings)
cargo clippy --workspace -- -D warnings

# Install local hooks: fmt on commit, rust-analyzer + clippy on push
bash scripts/install-hooks.sh

# Check facade with all features
cargo check -p rusty-modbus --features full --examples

# License/advisory checks
cargo deny check

# Fast benchmark smoke: codec microbenches + single-connection pipelined TCP
scripts/bench-local.sh smoke

# Focused benchmark runs
scripts/bench-local.sh codec --quick --noplot
scripts/bench-local.sh tcp-pipelined --quick --noplot
scripts/bench-local.sh stress --duration 10 --clients 1 --in-flight 8 --operation mixed --json

# Docker benchmark target
scripts/docker-bench.sh --duration 5 --clients 1 --in-flight 8 --json

# Docker image validation
scripts/docker-ci.sh

# Comparable stress matrix: local release + Alpine Docker + distroless Docker
scripts/bench-suite.sh all

# Local-only / Docker-only matrices
scripts/bench-suite.sh local
scripts/bench-suite.sh docker

# Full benchmark suite
scripts/bench-local.sh all --quick --noplot
```

See [benchmarks.md](benchmarks.md) for the current local stress-test baseline
and follow-up benchmark targets.

Release flow:

1. Work lands on `dev` through ordinary PRs.
2. Releases are cut by opening a `dev` -> `main` PR and passing `release.yml`.
3. A `v0.1.0` tag on `main` triggers `publish.yml`, which publishes crates in
   dependency order, publishes Python distributions to PyPI, and builds CLI
   release binaries.

Minimum Rust version: 1.95 (pinned in `rust-toolchain.toml`)

## License

MIT
