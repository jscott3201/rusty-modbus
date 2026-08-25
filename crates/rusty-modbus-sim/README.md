# rusty-modbus-sim

`rusty-modbus-sim` provides a Modbus/TCP simulator library and a binary that
loads one YAML device configuration. The current schema supports static
register and bit maps. Dynamic updates and fault injection are rejected.

The [simulator profile](../../docs/conformance/ledger.md#profile-simulator) uses
the existing [TCP server](../../docs/conformance/ledger.md#profile-tcp-server).
The ledger records repository evidence and open profile gaps for both.

## Run

From a workspace checkout:

```bash
cargo run -p rusty-modbus-sim -- crates/rusty-modbus-sim/examples/basic.yaml
```

From crates.io:

```bash
cargo install rusty-modbus-sim
rusty-modbus-sim device.yaml
```

The only arguments are one configuration path and `-h`/`--help`.

## Process output

After the listener binds, stdout receives one flushed readiness record:

```text
RUSTY_MODBUS_SIM_READY address=<SocketAddr> unit_id=<decimal u8>
```

The record contains three ASCII-space-separated fields in that order and no
additional fields. `address` uses Rust's `SocketAddr` display form, so an IPv6
address includes brackets. A configured port of zero is replaced by the bound
port in this record.

On Unix, SIGINT or SIGTERM starts the bounded server stop path. Other platforms
use Ctrl-C. After stop completes, stdout receives:

```text
RUSTY_MODBUS_SIM_STOPPED
```

Both records are newline-terminated. Diagnostics use stderr. Configuration and
bind failures do not emit a readiness record.

## Configuration rules

- Unit IDs 1 through 247 and direct TCP device ID 255 are accepted. Broadcast 0
  and reserved IDs 248 through 254 are rejected.
- `listen_addr` must parse as a `SocketAddr`; `127.0.0.1:0` requests an
  ephemeral port.
- Unknown and duplicate YAML fields are rejected, including nested fields.
- Every block has a nonzero count, fits the 16-bit address space, and has no
  more initial values than its count.
- Blocks in the same Modbus table cannot overlap. Adjacent blocks and equal
  ranges in different tables are valid.
- Register blocks use `mode: static`, `min: 0`, and `max: 65535`. These are the
  defaults when omitted. `random`, `increment`, other bounds, and nonempty
  `faults` lists are rejected.
- Missing initial values remain zero or `false`.

See [`examples/basic.yaml`](examples/basic.yaml) for a complete static device.

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
