# rusty-modbus-pool

Modbus connection pooling with idle eviction and reconnect backoff

Pool evidence and recovery gaps are tracked under the
[TCP client](../../docs/conformance/ledger.md#profile-tcp-client) and
[gateway](../../docs/conformance/ledger.md#profile-gateway) profiles.

- 📖 [API documentation](https://docs.rs/rusty-modbus-pool)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## Manual lease invalidation

Checked-out connections return to the idle pool on drop by default. Callers must
call `PooledConnection::invalidate` after a timeout or cancellation where I/O
may have occurred, or whenever transport, framing, or protocol behavior leaves
stream synchronization ambiguous and the lease must not be reused.

Invalidation immediately releases the lease's pool capacity and retires its TCP
connection instead of returning it to idle. The first caller-supplied
`LeaseInvalidationReason` is retained and later invalidation calls are no-ops.

This mechanism is manual only. The pool does not automatically detect errors or
cancellation, probe liveness, infer connection health, or prove protocol stream
synchronization. It therefore does not by itself resolve the tracked F-017 and
F-018 recovery gaps.

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
