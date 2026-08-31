# rusty-modbus-tcp

Modbus/TCP transport — split sink/stream traits and TCP implementation

Transport evidence and remaining gaps are scoped to the [TCP client](../../docs/conformance/ledger.md#profile-tcp-client),
[TCP server](../../docs/conformance/ledger.md#profile-tcp-server),
[gateway](../../docs/conformance/ledger.md#profile-gateway),
[Modbus/TCP Security](../../docs/conformance/ledger.md#profile-modbus-security), and
[simulator](../../docs/conformance/ledger.md#profile-simulator) profiles.

- 📖 [API documentation](https://docs.rs/rusty-modbus-tcp)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## Passive idle observation

`inspect_idle_tcp` consumes and reconstructs a matching `TcpSink` and
`TcpRecvStream` while preserving their socket, codec state, buffered data, and
timeout settings. It reports decoder-buffered bytes, immediately socket-readable
bytes, peer EOF, or a bounded socket error kind without consuming receive bytes,
sending data, or waiting for readiness. Mismatched halves are returned unchanged
with a defensive classification.

The result is only an instantaneous passive observation. In particular,
`NoAdverseSignal` does not prove peer liveness, Modbus protocol synchronization,
or future silence, and input can race with the observation.

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
