# ADR 0003: Compound register operations use explicit atomic callbacks

Status: accepted

## Decision

The server routes FC 0x16 and FC 0x17 through
`DataStore::atomic_mask_write_register` and
`DataStore::atomic_read_write_registers_be`. Their defaults return Illegal
Function without calling ordinary register methods. Existing `DataStore`
implementations remain source-compatible and must opt in before serving either
function.

The FC 0x17 hook accepts the write values and response buffer in big-endian wire
order. The handler validates the request envelope and allocates the complete
response before invoking the hook. It calls the hook once and rejects a returned
register count that differs from the requested read quantity.

`InMemoryStore` implements FC 0x16 with one holding-register write guard. Its FC
0x17 implementation validates the input, output, and both configured ranges
before mutation, then performs the write and encodes the post-write read while
holding one write guard. Overlapping ranges therefore observe the new values.

Python-backed stores opt in through `atomic_mask_write_register` and
`atomic_read_write_registers`. The Python object owns synchronization,
transaction, and cancellation behavior. The adapter does not add a global lock
or rely on GIL serialization; free-threaded Python remains supported.

## Alternatives

Composing the existing read and write methods was rejected because another
request can run between awaits. A server-wide request lock was rejected because
it would serialize unrelated stores and addresses. Generic rollback was rejected
because an external backend may commit before cancellation or response failure,
and the server cannot reverse an arbitrary side effect.

## Migration

Rust stores that support FC 0x16 or FC 0x17 implement the corresponding atomic
hook. Python stores implement the callbacks described by
`AtomicCompoundDataStore`. Stores without those methods continue to serve their
existing functions and return exception `0x01` for FC 0x16 and FC 0x17.

The FC 0x17 callback must return exactly the requested number of 16-bit values.
Python callbacks can raise the existing typed Modbus exceptions to select the
wire exception code.

## Consequences

The built-in store gives each compound operation one lock-defined linearization
point without serializing other data tables. Custom stores define their own
linearization and cancellation boundaries. A successful commit is not rolled
back if the callback later returns an invalid result or response delivery fails.

FC 0x15 is unchanged. Its complete request is validated before the first backend
write, but backend failure, cancellation, and concurrency do not provide rollback
or a transaction spanning file-record groups.
