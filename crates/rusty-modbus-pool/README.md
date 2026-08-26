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

Direct callers using a checked-out lease's raw TCP halves can opt into a pure,
conservative suggestion for a `TransportError`. Scope the transport borrow
before deciding whether to invalidate the lease:

```rust
use rusty_modbus_pool::LeaseInvalidationReason;
use rusty_modbus_tcp::transport::TransportSink;

let suggested_reason = {
    let send_result = lease.sink().send(frame).await;
    send_result
        .as_ref()
        .err()
        .and_then(LeaseInvalidationReason::suggested_for_transport_error)
};

if let Some(reason) = suggested_reason {
    lease.invalidate(reason);
}
```

This mechanism and classifier remain manual only. The helper neither invalidates
the lease nor mutates pool state, and a `None` suggestion does not prove health,
reusability, liveness, or protocol stream synchronization. Cancellation may
require an explicit `LeaseInvalidationReason::Cancelled` because it may not
produce a `TransportError`. This API therefore does not by itself resolve the
tracked F-017 and F-018 recovery gaps.

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
