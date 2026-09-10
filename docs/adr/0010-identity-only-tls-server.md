# ADR 0010: Bounded sequential identity-only TLS serving foundation

Status: accepted

## Owner decision and scope

The owner explicitly chose **TLS foundation first (Recommended)** while PR-302
pipelining/performance acceptance remains deferred. This splits a Rust-only,
bounded sequential mTLS foundation from full PR-501 ahead of PR-302; it does not
waive PR-302's gate or complete full PR-501/PR-502 qualifications.

The profile is **identity-only**. A trusted client certificate authenticates a
connection, but every CA-authenticated peer has access to the configured
`DataStore`. No role-based request authorization is performed. Startup rejects
`require_client_cert=false` and any configured authorization callback, including
an allow-all callback, before loading certificates or binding. An unsupported
policy must not appear to be enforced or be silently ignored.

## Boundaries and reuse

- `TcpServerListener::accept_socket` performs the existing ACL, connection
  reservation and nodelay setup before returning an unframed socket and guard.
  The old framed `accept` delegates to it; setup errors still reclaim capacity.
  This socket is plaintext, not an authenticated TLS connection.
- `TlsServerAcceptor` uses the existing TLS 1.3/512-byte-fragment rustls builder,
  upgrades a caller-owned socket, and returns TLS MBAP halves with explicit I/O
  timeouts and owned optional role metadata. Caller-owned deadlines/cancellation
  and admission guards remain outside this low-level helper.
- `TlsServerListener` retains its existing API/defaults and unenforced low-level
  callback/connection-limit behavior. `TlsServerConfig::authorize` still has its
  original allow-all default. The new high-level restrictions do not silently
  harden or break those existing APIs.
- The existing sequential connection body is transport-generic and shared;
  request dispatch, Unit/broadcast rules, PDU bounds, custom hooks and datastore
  atomic operations are not reimplemented. The existing lifecycle coordinator,
  one absolute drain deadline, joins and bounded accept-error backoff are reused.

## New API and resource contract

`TlsModbusServerConfig::new(tls_config)` requires an explicit certificate config.
It exposes only enforced inbound controls, not unused connect/keepalive/port or
transaction-concurrency settings. Defaults are port 802, Unit ID 1, 64 connections,
16 handshakes, 5s handshake timeout, 10s shutdown timeout, 30s read/write timeouts
and nodelay true. Optional read/write `None` is allowed. Required limits and
configured timeouts must be nonzero; capacities above `Semaphore::MAX_PERMITS`
are rejected with typed errors rather than panicking.

The effective connection cap is `min(server.max_connections,
tls.max_connections)`, including every admitted TCP socket throughout handshake
and authenticated serving. The effective handshake cap is the minimum of its
configured bound and that connection cap. Saturation rejects without spawning
waiting tasks. A stalled handshake cannot monopolize another available slot.
Timeout, failure, cancellation, success and Drop all reclaim handshake permits;
the TCP guard remains until the session ends.

Stop seals admission, cancels handshakes, drains admitted requests and aborts/
joins unfinished yielding tasks at the shared deadline. Cancelling a stop caller
does not cancel the coordinator. Drop is nonwaiting abort; neither immediate
rebinding nor timely abort of a non-yielding datastore future is promised.

Metrics distinguish TCP admission from TLS success. The new snapshot embeds
unchanged `ServerMetrics` and adds active handshake/session counts and cumulative
handshake outcomes/rejections. Snapshots are nontransactional, and no raw
certificate/key material is logged.

## Feature graph and compatibility

The layer-3 server gains an optional normal dependency on layer-2 TLS through
feature `tls`. TLS does not gain a normal dependency on the server. Existing
development edges remain test-only. Facade `tls` weakly forwards to
`rusty-modbus-server?/tls`: `server` + `tls`/`full` exposes the new API, whereas
TLS-only does not activate the server. Default TCP and server-only normal graphs
do not acquire TLS. No dependency version or default feature changes are needed.

## Explicit residuals

- The role extractor's permissive absent/unparseable-to-`None` behavior remains.
  This profile does not prove strict malformed-role rejection or complete
  authenticated per-request peer context.
- Per-request role policy, authorization-denial responses, full Security-profile
  claims, certificate lifecycle and remaining PR-501/502/503 qualification remain
  separate. F-014/F-015 are not closed by this foundation.
- Python, CLI, simulator and gateway serving surfaces are unchanged. TCP defaults
  and its existing non-operative `max_transactions` setting remain unchanged.
- No pipelining, performance measurement, controlled-runner acceptance or release
  qualification is claimed. Host contention still blocks performance acceptance.

The supported caller workflow and detailed limits are in
[docs/api.md](../api.md#identity-only-tls-server).
