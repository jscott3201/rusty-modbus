# rusty-modbus-client

Pipelined async Modbus client

Client evidence is transport-specific: [TCP client](../../docs/conformance/ledger.md#profile-tcp-client),
[physical RTU client](../../docs/conformance/ledger.md#profile-physical-rtu-client),
[Modbus/TCP Security](../../docs/conformance/ledger.md#profile-modbus-security), and
[RTU-over-TCP extension](../../docs/conformance/ledger.md#profile-rtu-over-tcp-extension).
Each profile lists its evidence level and compatibility deviations.

- 📖 [API documentation](https://docs.rs/rusty-modbus-client)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## Retry and deadline behavior

`ClientConfig::timeout` bounds each request attempt after semaphore admission.
Waiting for admission is not timed; the bounded logical-request envelope starts
when a permit is acquired. The client retries response timeouts and transport
timeouts only for replay-safe reads. Typed writes are not replayed after an
ambiguous timeout or transport failure; either error may occur after the server
applied the write. A configured Server Device Busy (`0x06`) response remains
retryable for reads and writes.

Acknowledge (`0x05`) is terminal. It is returned as
`ClientError::Exception(ExceptionResponse)` to show that the server accepted the
request and is still processing it. The application owns any completion check;
the client does not report Acknowledge as success or replay the request.

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
