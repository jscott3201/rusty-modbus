# rusty-modbus-rtu

Modbus RTU transport — serial and RTU-over-TCP

The [physical RTU client](../../docs/conformance/ledger.md#profile-physical-rtu-client)
has listed timing and serial-format deviations. A first-party
[physical RTU responder](../../docs/conformance/ledger.md#profile-physical-rtu-responder)
is not implemented. [Gateway](../../docs/conformance/ledger.md#profile-gateway)
evidence is separate from the
[RTU-over-TCP extension](../../docs/conformance/ledger.md#profile-rtu-over-tcp-extension),
which has no physical-line or MBAP claim.

- 📖 [API documentation](https://docs.rs/rusty-modbus-rtu)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
