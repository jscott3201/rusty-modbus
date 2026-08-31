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

## Passive idle TCP validation

Before checkout charges an idle entry active, the pool passively inspects each
same-address candidate. The periodic health sweep performs the same inspection
for every idle priority and non-priority entry. It checks bytes already buffered
by the framed decoder, then performs one non-consuming `poll_peek` on the socket.
Queued input is adverse even when it could decode as a valid Modbus frame. Peer
EOF, a socket error, and defensively mismatched transport halves are also
adverse. The exact idle connection is retired without consuming or logging its
bytes; checkout continues to another candidate or a normal new reservation.

Priority connections remain protected from age and capacity eviction, but a
priority idle transport with a known adverse signal is retired. Clean/unknown
priority entries remain idle regardless of age. Clean/unknown non-priority
entries remain only until their normal idle timeout or LRU capacity eviction.
Passive retirement preserves the separate priority and non-priority budgets and
wakes capacity waiters when capacity may have become available.

This is an instantaneous observation only: it sends no probe or other socket
write, consumes no receive byte, and never waits for readiness. A no-adverse-
signal result does not prove peer liveness or protocol synchronization. Input can
arrive after validation and before or during the next borrow. In particular, a
late response that is already observable before checkout is retired rather than
handed to the next borrower, but a response racing after validation remains
possible for idle entries created by pre-connect or verdict-gated client return.
Raw lease drop no longer creates such an idle entry. This work does not close
F-017, F-018, or TCP-013 and does not add automatic reconnect or backoff behavior.

Each passive retirement emits one `DEBUG` event with target
`rusty_modbus_pool::idle_validation` and message
`idle_tcp_connection_passively_retired`. Its bounded fields are `reason`
(`queued_input`, `peer_closed`, `socket_error`, `mismatched_halves`, or `other`),
`trigger` (`checkout` or `health_sweep`), and boolean `is_priority`. Addresses,
payload bytes, Unit IDs, and error text are not recorded.

## Raw lease retirement and manual classification

**Compatibility break:** dropping a checked-out raw `PooledConnection` now
always retires its exact TCP transport and releases its active capacity charge.
Raw drop never inserts the connection into idle, even when the lease was never
used or a raw send/receive completed successfully. A later `get` therefore opens
a fresh TCP connection unless an independently created idle entry is available.
Raw callers should expect more TCP handshakes and connection churn.

`PooledConnection::invalidate` remains useful when a caller wants to retire
immediately and classify an observed timeout, cancellation, transport failure,
or protocol ambiguity. The first caller-supplied `LeaseInvalidationReason` is
retained and later invalidation calls and drop are no-ops for accounting.
Ordinary raw drop does not invent an invalidation reason.

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
require an explicit `LeaseInvalidationReason::Cancelled` for classification
because it may not produce a `TransportError`. Raw drop still retires when no
classification is recorded. This API does not by itself resolve the tracked
F-017 and F-018 recovery gaps.

Each ordinary raw drop retirement emits one `DEBUG` event with target
`rusty_modbus_pool::raw_lease` and message `pooled_raw_connection_retired`. Its
bounded fields are `trigger` (`drop`), `raw_accessed`, and `is_priority`. No
address, payload, Unit ID, invalidation reason, or error text is recorded.

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
client dependency and both handoff APIs are absent. Raw drop retirement still
applies. This handoff remains available after direct raw-half access because it
can never return idle; prior raw access does not make starting another operation
semantically safe. The facade crate's `pool` feature enables the handoffs.

## Verdict-gated reusable client handoff

The same opt-in `client` feature also provides a narrower reusable wrapper:

```rust,no_run
use rusty_modbus_pool::{ClientConfig, ConnectionPool, PooledClientReturnOutcome};

# async fn example(pool: ConnectionPool, addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
let session = pool
    .get(addr)
    .await?
    .into_reusable_client(ClientConfig::default())?;

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

`PooledConnection::into_reusable_client` accepts only a pristine lease and keeps
ownership in the pool while exposing borrowed access to the normal high-level
client API. Calling raw `sink()` or `stream()` first permanently disqualifies the
lease and returns `ReusableClientHandoffError::RawTransportAccessed`; the rejected
lease is retired. `addr()` does not disqualify it. An already-retired lease
returns `ReusableClientHandoffError::LeaseUnavailable` rather than panicking.

Only consuming `shutdown_and_return` can reinsert the TCP connection, and only
after graceful shutdown joins all client tasks and the final local
`SessionReuseVerdict` is exactly `ReuseEligible`. Timeout, post-dispatch
cancellation, malformed, mismatched, unknown, duplicate, or typed-invalid
responses, reader failure, abort, an incomplete shutdown, wrapper drop, recovery
failure, and pool shutdown all retire the connection and release capacity
without inserting it into idle.

This is a local synchronization-safety contract, not a peer-health guarantee. It
assumes a conforming peer does not invent a future duplicate after all valid
requests complete. There is no active probe, quiet-period test, peer-liveness
proof, or guarantee of permanent future silence. This opt-in return path does
not close the tracked F-017/F-018 recovery gaps. The existing
`into_retiring_client` method remains an always-retiring option. For connection
reuse, enable `client`, call `into_reusable_client(...)?` before any raw-half
access, perform operations through `PooledClientSession::client`, then consume
the wrapper with `shutdown_and_return`. Raw drop always retires.

Each `PooledClientSession` lifecycle emits exactly one structured tracing event
with target `rusty_modbus_pool::client_handoff` and message
`pooled_client_session_completed`. Expected outcomes use `DEBUG`; an internal
`transport_recovery_failed` outcome uses `WARN`. For example, an application can
enable these events with an `EnvFilter` directive such as
`rusty_modbus_pool::client_handoff=debug`.

The event fields are bounded labels: `outcome` is `returned_to_idle`, `retired`,
`pool_shutting_down`, or `transport_recovery_failed`; `trigger` is
`shutdown_and_return` or `wrapper_drop`; `verdict` is `reuse_eligible`,
`not_quiescent`, or `retire`; `retirement_reason` is a stable snake-case reason
label (`none` when inapplicable and `other` for a future unknown reason); and
`is_priority` is a boolean. No request, address, Unit ID, error text, or other
high-cardinality value is recorded.

Tracing is observability only. It adds no public counters, health probe,
liveness proof, automatic raw-lease invalidation, or recovery/backoff policy,
and it does not close F-017 or F-018.

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
