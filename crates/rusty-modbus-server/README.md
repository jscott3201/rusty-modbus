# rusty-modbus-server

Async Modbus server with pluggable DataStore backend

The server implements the [TCP server profile](../../docs/conformance/ledger.md#profile-tcp-server).
A first-party [physical RTU responder](../../docs/conformance/ledger.md#profile-physical-rtu-responder)
is not implemented, and the [Modbus/TCP Security](../../docs/conformance/ledger.md#profile-modbus-security)
crate provides primitives rather than a composed secured server.

- 📖 [API documentation](https://docs.rs/rusty-modbus-server)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
