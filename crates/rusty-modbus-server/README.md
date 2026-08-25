# rusty-modbus-server

Async Modbus server with pluggable DataStore backend

The server implements the [TCP server profile](../../docs/conformance/ledger.md#profile-tcp-server).
A first-party [physical RTU responder](../../docs/conformance/ledger.md#profile-physical-rtu-responder)
is not implemented, and the [Modbus/TCP Security](../../docs/conformance/ledger.md#profile-modbus-security)
crate provides primitives rather than a composed secured server.

`ServerConfig::validate` rejects zero connection, transaction, and shutdown
limits before bind. `max_transactions` is not a runtime concurrency control:
each connection still processes one request at a time.

Rust `DataStore` implementations can override `handle_custom_function` for
non-standard function codes. The hook receives the Unit Identifier, raw
function code, and at most `MAX_PDU_SIZE - 1` request-data bytes. It writes only
vendor response data into a server-owned buffer of exactly that size; the server
prepends the original function code and owns MBAP framing. A length above the
buffer bound becomes Server Device Failure. The default returns Illegal
Function, and custom broadcasts are suppressed without invoking the hook.

This hook is not exposed through the Python bindings, CLI, or simulator. It is
trusted in-process code, not an authorization or isolation boundary. A panic
follows the normal datastore connection-task cleanup path, and a secured server
must authorize a request before custom dispatch.

`ModbusServer::stop` seals listener and request admission, drops the listener,
and lets admitted requests finish until `ServerConfig::shutdown_timeout`. It
returns `ShutdownOutcome::Drained` when all connection tasks finish or
`ShutdownOutcome::Forced` after aborting and joining the remainder. Concurrent
callers share one deadline and outcome. `ModbusServer::metrics` returns active
connection/request counts plus cumulative listener rejection and error counts.

Tokio task abort is cooperative. A datastore future that does not yield can
delay forced shutdown beyond the deadline. Dropping `ModbusServer` is a
synchronous abort request; it does not wait for graceful completion or guarantee
that the listen address can be rebound immediately.

- 📖 [API documentation](https://docs.rs/rusty-modbus-server)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
