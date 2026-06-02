# Rusty Modbus

A complete Modbus protocol stack written in Rust, covering TCP, RTU, and TLS transports with pipelined async client, pluggable server, TCP-RTU gateway, connection pooling, YAML-driven simulator, and CLI tool.

[![CI](https://github.com/jscott3201/rusty-modbus/actions/workflows/ci.yml/badge.svg)](https://github.com/jscott3201/rusty-modbus/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Features

- **3 transports** — Modbus/TCP, RTU (serial + RTU-over-TCP), TLS 1.3 (mutual auth)
- **Pipelined async client** — 16-slot transaction ring with background reader task
- **Pluggable server** — async `DataStore` trait for custom register backends
- **TCP-RTU gateway** — transparent protocol translation bridge
- **Connection pooling** — two-pool architecture with health checks and backoff
- **YAML simulator** — device profiles with scenario-driven register behavior
- **CLI tool** — read/write/shell/discover commands with JSON output
- **Spec conformance** — 537+ tests, validation order per V1.1b3 section 4.5
- **`no_std` foundation** — types and codec crates work without allocator
- **Zero `unsafe`** — `#![forbid(unsafe_code)]` on all crates
- **Zero clippy warnings**, CI on Linux/macOS/Windows

## Quick Start

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

## CLI Tool

```bash
cargo install rusty-modbus-cli

# Read holding registers
modbus read -H 192.168.1.100 holding 0 10

# Write a single register
modbus write -H 192.168.1.100 register 0 0x1234

# Interactive shell
modbus shell -H 192.168.1.100

# Discover devices on a subnet
modbus discover --range 192.168.1.0/24

# JSON output (for scripting)
modbus read -H 192.168.1.100 holding 0 10 --format json
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
  rusty-modbus-gateway/     TCP <-> RTU bridge
  rusty-modbus-sim/         YAML simulator + device profiles
  rusty-modbus-cli/         CLI binary (read/write/shell/discover)
  rusty-modbus/             Facade crate with feature flags
  rusty-modbus-conformance/ Spec compliance test suite
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

## Supported Function Codes

The client exposes **14** typed function codes; the server handles **11**.

| Function Code | Name | Client | Server |
|---------------|------|--------|--------|
| 0x01 | Read Coils | yes | yes |
| 0x02 | Read Discrete Inputs | yes | yes |
| 0x03 | Read Holding Registers | yes | yes |
| 0x04 | Read Input Registers | yes | yes |
| 0x05 | Write Single Coil | yes | yes |
| 0x06 | Write Single Register | yes | yes |
| 0x0F | Write Multiple Coils | yes | yes |
| 0x10 | Write Multiple Registers | yes | yes |
| 0x14 | Read File Record | yes | no |
| 0x15 | Write File Record | yes | no |
| 0x16 | Mask Write Register | yes | yes |
| 0x17 | Read/Write Multiple Registers | yes | yes |
| 0x18 | Read FIFO Queue | yes | no |
| 0x2B/0x0E | Read Device Identification (MEI) | yes | yes |

File Record (0x14/0x15) and FIFO Queue (0x18) are currently client-only.

**Not yet implemented:** Read Exception Status (0x07), Diagnostics (0x08),
Get Comm Event Counter/Log (0x0B/0x0C), Report Server ID (0x11).

## Development

```bash
# Build entire workspace
cargo build --workspace

# Run all tests (537+)
cargo test --workspace

# Lint (must be zero warnings)
cargo clippy --workspace -- -D warnings

# Check facade with all features
cargo check -p rusty-modbus --features full --examples

# License/advisory checks
cargo deny check

# Benchmarks
cargo bench -p rusty-modbus-benchmarks --bench codec
cargo bench -p rusty-modbus-benchmarks --bench tcp_latency
cargo run -p rusty-modbus-benchmarks --bin stress-test -- --help
```

Minimum Rust version: 1.95 (pinned in `rust-toolchain.toml`)

## License

MIT
