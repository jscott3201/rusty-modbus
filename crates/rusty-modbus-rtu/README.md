# rusty-modbus-rtu

Modbus RTU transport — serial and RTU-over-TCP

## Physical serial configuration

`RtuConfig` retains the original 9600/8N1 default, and
`SerialTransport::open` remains permissive for compatibility. New physical
serial integrations should construct
`StrictRtuConfig` and call `SerialTransport::open_strict`. The strict path
accepts only 8E1, 8O1, or 8N2; rejects a zero baud rate; rounds calculated RTU
timers up to whole nanoseconds; and validates Unit Identifiers by direction.

The returned `SerialSink` and `SerialRecvStream` expose the immutable
`ResolvedRtuConfig` used to open the port. It includes the concrete serial
driver settings, response timeout, character time, t1.5, t3.5, and whether the
timers were character-calculated or use the fixed recommendation above 19,200
bit/s.

## Timestamp-driven frame assembly

`RtuFrameAssembler` is a runtime-independent receive core for callers that
already have trustworthy monotonic timestamps for each byte. It retains one
fixed 256-byte candidate, treats gaps above t1.5 and below t3.5 as corruption,
uses tokenized t3.5 deadlines, and emits an inline owned ADU only after checking
the complete candidate's length and CRC. Timing comes from `RtuTiming` or
directly from `ResolvedRtuConfig`.

The core is not connected to `SerialTransport`, `tokio-serial`, or an async read
adapter. Serial read-completion timestamps cannot recover byte timing hidden by
an OS or USB buffer, so this API does not establish physical receive framing or
read-chunk invariance. Timestamp-source and adapter integration remain open.

Enable the crate's `serial` feature for physical ports. Through the
`rusty-modbus` facade, use `rtu-serial`; the smaller `rtu` feature does not pull
in `tokio-serial`.

The [physical RTU client](../../docs/conformance/ledger.md#profile-physical-rtu-client)
retains listed receive-framing and legacy compatibility deviations. A first-party
[physical RTU responder](../../docs/conformance/ledger.md#profile-physical-rtu-responder)
is not implemented. [Gateway](../../docs/conformance/ledger.md#profile-gateway)
evidence is separate from the
[RTU-over-TCP extension](../../docs/conformance/ledger.md#profile-rtu-over-tcp-extension),
which has no physical-line or MBAP claim.

- 📖 [API documentation](https://docs.rs/rusty-modbus-rtu)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
