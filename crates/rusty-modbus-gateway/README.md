# rusty-modbus-gateway

Modbus TCP-to-RTU-over-TCP gateway bridge

The [gateway profile](../../docs/conformance/ledger.md#profile-gateway) records
implemented routing evidence. The current backend uses RTU-over-TCP; a physical
serial gateway is not implemented.

Each `RouteEntry` selects its RTU-over-TCP response framing policy. Construct
routes with `RouteEntry::new(range, address)` to retain compatibility framing,
or chain `with_rtu_over_tcp_framing_policy(FunctionAwareStrict)` to opt in. This
constructor replaces pre-0.2 struct literals, which now require the policy
field. Broadcast forwarding remains send-only and encoding is policy-independent.

- 📖 [API documentation](https://docs.rs/rusty-modbus-gateway)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
