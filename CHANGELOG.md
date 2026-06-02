# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/jscott3201/rusty-modbus/commits/main
