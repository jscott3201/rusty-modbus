# rusty-modbus-tls

Modbus/TCP Security — TLS 1.3 transport; mutual certificate authentication is
enabled by default and has an explicit compatibility opt-out

The [Modbus/TCP Security profile](../../docs/conformance/ledger.md#profile-modbus-security)
records evidence for TLS transport, the authentication modes, and role parsing.
These primitives do not by themselves provide a composed secured server or
interoperability evidence.

`TlsServerAcceptor::new(&TlsServerConfig)` reuses the existing TLS 1.3 and
512-byte fragment configuration. Its `accept(admitted_tcp_socket, read_timeout,
write_timeout)` consumes a caller-owned socket, preserves socket options and
returns TLS MBAP halves plus the existing owned optional role. The caller owns
admission limits, handshake deadlines/cancellation and authorization. Cancelling
the upgrade future closes its socket. Returned frame read/write timeouts are
explicit; role parsing alone does not authorize requests or prove strict role
extension validation.

`TlsServerListener` retains its previous API/default behavior: nodelay true,
unbounded handshake wait, no frame I/O timeouts, and no built-in enforcement of
`TlsServerConfig.max_connections` or `authz_callback`. The configuration's
`authorize` allow-all default and explicit non-mTLS compatibility option remain
unchanged.

For bounded **identity-only** serving, enable `rusty-modbus-server/tls` and use
`TlsModbusServer`. That new profile requires mTLS and rejects callbacks at
startup; all CA-authenticated peers have datastore access without role policy.
It does not make these low-level primitives a role-authorized serving runtime.
See [ADR 0010](../../docs/adr/0010-identity-only-tls-server.md).

- 📖 [API documentation](https://docs.rs/rusty-modbus-tls)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
