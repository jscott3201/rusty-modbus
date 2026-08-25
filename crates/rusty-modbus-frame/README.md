# rusty-modbus-frame

Modbus framing — MBAP/RTU codecs, CRC-16, owned Bytes types

Framing evidence is tracked under the [TCP client](../../docs/conformance/ledger.md#profile-tcp-client),
[TCP server](../../docs/conformance/ledger.md#profile-tcp-server),
[physical RTU client](../../docs/conformance/ledger.md#profile-physical-rtu-client),
[gateway](../../docs/conformance/ledger.md#profile-gateway),
[Modbus/TCP Security](../../docs/conformance/ledger.md#profile-modbus-security), and
[RTU-over-TCP extension](../../docs/conformance/ledger.md#profile-rtu-over-tcp-extension)
profiles. RTU-over-TCP does not inherit physical-line or MBAP claims.

## RTU-over-TCP framing policies

Bare/default `RtuOverTcpCodec` uses the named `CrcScanCompatibility` policy and
emits the first CRC-valid prefix. `RtuOverTcpCodec::with_policy` can opt into
`FunctionAwareStrict` with an explicit incoming `Request` or `Response`
direction. Strict framing derives one boundary for supported self-delimiting
standard forms and never falls back to CRC scanning. Both policies return a
terminal error for malformed input at the 256-byte ADU bound; callers must close
the framed connection rather than attempt resynchronization.

See [ADR 0004](../../docs/adr/0004-rtu-over-tcp-framing-policy.md) for supported
forms, unsupported diagnostics/MEI/custom forms, and the extension evidence
boundary.

- 📖 [API documentation](https://docs.rs/rusty-modbus-frame)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
