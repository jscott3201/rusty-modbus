# rusty-modbus-server

Async Modbus server with pluggable DataStore backend

The server implements the [TCP server profile](../../docs/conformance/ledger.md#profile-tcp-server).
A first-party [physical RTU responder](../../docs/conformance/ledger.md#profile-physical-rtu-responder)
is not implemented. The opt-in TLS foundation below does not complete the
[Modbus/TCP Security profile](../../docs/conformance/ledger.md#profile-modbus-security).

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

## Opt-in identity-only TLS foundation

Enable this crate's `tls` feature and use `TlsModbusServer<S>` with
`TlsModbusServerConfig::new(rusty_modbus_tls::TlsServerConfig { ... })`.
Certificate/key/CA configuration is explicit. The constructor defaults to
`0.0.0.0:802`, Unit ID 1, connection limit 64, handshake limit 16, handshake
timeout 5s, shutdown timeout 10s, read/write timeouts 30s, and `TCP_NODELAY=true`.

**Every peer authenticated by the configured client CA has access to the
configured DataStore. There is no per-request role authorization.** Startup
rejects `require_client_cert=false` and **any** `authz_callback=Some`, even an
allow-all callback, before certificate loading/bind. It never silently ignores
an authorization policy. Missing/unparseable role metadata does not deny a
trusted peer under this identity-only profile; the legacy role extractor is not
a strict malformed-role validator.

`TlsServerStartError` retains typed configuration, TLS-material and transport
errors. `IdentityTlsConfigError` identifies unsupported profiles, zero limits/
configured timeouts, and capacities above `tokio::sync::Semaphore::MAX_PERMITS`.
Read/write `None` is permitted; handshake/shutdown timeouts must remain nonzero.
The effective connection limit is
`min(config.max_connections, config.tls.max_connections)` and counts **all**
admitted TCP sockets, including handshakes and authenticated sessions. The
handshake cap is `min(config.max_handshakes, effective_connection_limit)`;
excess handshakes are rejected without a waiting task queue. ACL and nodelay
setup happen before TLS. The existing TLS 1.3/512-byte fragment builder is reused.

The same sequential request runner handles standard/custom PDUs, Unit IDs,
broadcasts, exceptions, PDU bounds and datastore-owned atomic operations. This
new config deliberately has no transaction-concurrency, outbound-connect,
keepalive or secondary-port knobs that would be accepted but ignored. Existing
TCP `ServerConfig` defaults and `max_transactions` behavior are unchanged.

`start`, `local_addr`, `store`, `metrics` and `stop -> ShutdownOutcome` mirror the
TCP server's ownership model. Stop seals admission, cancels pending handshakes,
drains admitted requests against the single shared absolute shutdown deadline,
then aborts/joins unfinished yielding tasks. Cancelling a stop caller does not
cancel that coordinator. Drop is nonwaiting cancellation, not graceful shutdown
or guaranteed immediate rebind. Non-yielding store futures retain Tokio's abort
limitations.

`TlsServerMetrics.server` contains the existing snapshot: accepted/active
connections are TCP admission counts, **not authenticated-client counts**.
Separate counters expose active handshakes/sessions, handshake starts/successes/
failures/timeouts/cancellations and handshake-cap rejections. Snapshots are
nontransactional. Low-level TLS listener behavior, including its callback and
limit non-enforcement, is unchanged. This Rust-only foundation adds no Python,
CLI, simulator or gateway serving API, no pipelining, and no full role-authorized
Security-profile claim. See [ADR 0010](../../docs/adr/0010-identity-only-tls-server.md)
and [API details](../../docs/api.md#identity-only-tls-server).

- 📖 [API documentation](https://docs.rs/rusty-modbus-server)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
