# rusty-modbus-pool

Modbus connection pooling with idle eviction and reconnect backoff

Pool evidence and recovery gaps are tracked under the
[TCP client](../../docs/conformance/ledger.md#profile-tcp-client) and
[gateway](../../docs/conformance/ledger.md#profile-gateway) profiles.

- 📖 [API documentation](https://docs.rs/rusty-modbus-pool)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## Capacity acquisition

`ConnectionPool::get` remains fail-fast: when the relevant non-priority or
per-device priority budget is full and no idle entry can be reused or evicted,
it returns `PoolError::Exhausted`. Call
`ConnectionPool::get_with_acquisition_timeout` to opt into waiting up to one
fixed deadline for that capacity instead.

The acquisition timeout covers only time spent waiting for pool capacity. It
ends once capacity is reserved and does not wrap TCP connection establishment,
transport I/O, or the independent `TcpConfig::connect_timeout`. A zero duration
still performs one immediate idle-reuse or reservation attempt before returning
`PoolError::Timeout` for a full budget.

If a supplied duration is too large to represent as an absolute deadline, the
same initial acquisition attempt still runs. When the relevant budget remains
full after a final state check, the method returns `PoolError::Timeout` instead
of panicking, silently shortening the duration, or treating it as an unlimited
wait. Representable durations retain their requested absolute deadline.

Capacity-change broadcasts are retry hints rather than permits, so waiters may
wake spuriously or because another pool budget changed. Every waiter rechecks
the exact pool state, and no fairness or FIFO order is guaranteed. Cancelling a
capacity wait changes no accounting; cancelling a later pending connection uses
the reservation guard to release its charge. Pool shutdown wakes blocked
waiters, which return `PoolError::ShuttingDown` when shutdown is observed.

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

## Retiring high-level client handoff

Enable the opt-in `client` feature to consume a checked-out raw TCP lease as a
high-level client:

```toml
rusty-modbus-pool = { version = "0.1.1", features = ["client"] }
```

```rust,no_run
use rusty_modbus_pool::{ClientConfig, ConnectionPool};

# async fn example(pool: ConnectionPool, addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
let client = pool
    .get(addr)
    .await?
    .into_retiring_client(ClientConfig::default());
# client.shutdown().await;
# Ok(())
# }
```

`PooledConnection::into_retiring_client` transfers both TCP halves and the
active capacity charge to the existing `ModbusClient` transaction engine. This
is a conservative borrower-isolation fence: after both client-owned halves are
gone, capacity is released exactly once, waiters are notified, and the
connection is retired instead of inserted into idle. It always retires,
including after a healthy session and during pool shutdown. Safe reuse of a
healthy client session is intentionally not provided.

The fence prevents a timed-out, cancelled, or delayed response from one handed-
off session from reaching a later pool borrower on the same TCP connection. It
does not actively probe liveness, prove stream synchronization, or close the
tracked F-017/F-018 recovery gaps. Without the `client` feature, the optional
client dependency and handoff API are absent, and existing raw lease behavior
is unchanged. The facade crate's `pool` feature enables this handoff.

## Verdict-gated reusable client handoff

The same opt-in `client` feature also provides a narrower reusable wrapper:

```rust,no_run
use rusty_modbus_pool::{ClientConfig, ConnectionPool, PooledClientReturnOutcome};

# async fn example(pool: ConnectionPool, addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
let session = pool
    .get(addr)
    .await?
    .into_reusable_client(ClientConfig::default());

let values = session
    .client()
    .read_holding_registers(rusty_modbus_types::UnitId(1), 0, 1)
    .await?;
assert_eq!(values.len(), 1);

match session.shutdown_and_return().await {
    PooledClientReturnOutcome::ReturnedToIdle => {}
    outcome => eprintln!("TCP session retired: {outcome:?}"),
}
# Ok(())
# }
```

`PooledConnection::into_reusable_client` keeps ownership in the pool while
exposing borrowed access to the normal high-level client API. Only consuming
`shutdown_and_return` can reinsert the TCP connection, and only after graceful
shutdown joins all client tasks and the final local `SessionReuseVerdict` is
exactly `ReuseEligible`. Timeout, post-dispatch cancellation, malformed,
mismatched, unknown, duplicate, or typed-invalid responses, reader failure,
abort, an incomplete shutdown, wrapper drop, recovery failure, and pool shutdown
all retire the connection and release capacity without inserting it into idle.

This is a local synchronization-safety contract, not a peer-health guarantee. It
assumes a conforming peer does not invent a future duplicate after all valid
requests complete. There is no active probe, quiet-period test, peer-liveness
proof, or guarantee of permanent future silence. This opt-in return path does
not close the tracked F-017/F-018 recovery gaps. The existing
`into_retiring_client` method remains the conservative always-retiring default.

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
