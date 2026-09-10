# rusty-modbus

Feature-gated facade for the workspace's Modbus components

Capability and limitation evidence is profile-scoped: [TCP client](../../docs/conformance/ledger.md#profile-tcp-client),
[TCP server](../../docs/conformance/ledger.md#profile-tcp-server),
[physical RTU client](../../docs/conformance/ledger.md#profile-physical-rtu-client),
[physical RTU responder](../../docs/conformance/ledger.md#profile-physical-rtu-responder),
[gateway](../../docs/conformance/ledger.md#profile-gateway),
[Modbus/TCP Security](../../docs/conformance/ledger.md#profile-modbus-security),
[simulator](../../docs/conformance/ledger.md#profile-simulator), and
[RTU-over-TCP extension](../../docs/conformance/ledger.md#profile-rtu-over-tcp-extension).

Enable **both** `server` and `tls` (or `full`) for
`rusty_modbus::server::TlsModbusServer`, `TlsModbusServerConfig`, and the
`rusty_modbus::TlsServer` alias. The `tls` feature weakly forwards to an already
enabled server dependency; TLS-only does not enable the server, and default TCP
or server-only normal builds do not acquire TLS dependencies.

This is a bounded, sequential **identity-only mTLS** foundation, not role
authorization: every CA-authenticated peer can access the configured datastore.
The new startup API rejects disabled client authentication and any authorization
callback. Existing TCP and low-level TLS APIs remain unchanged. Configuration,
metrics, shutdown limits and remaining profile gaps are in
[the API guide](../../docs/api.md#identity-only-tls-server).

- 📖 [API documentation](https://docs.rs/rusty-modbus)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
