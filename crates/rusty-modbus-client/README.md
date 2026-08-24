# rusty-modbus-client

Pipelined async Modbus client

Client evidence is transport-specific: [TCP client](../../docs/conformance/ledger.md#profile-tcp-client),
[physical RTU client](../../docs/conformance/ledger.md#profile-physical-rtu-client),
[Modbus/TCP Security](../../docs/conformance/ledger.md#profile-modbus-security), and
[RTU-over-TCP extension](../../docs/conformance/ledger.md#profile-rtu-over-tcp-extension).
Each profile lists its evidence level and compatibility deviations.

- 📖 [API documentation](https://docs.rs/rusty-modbus-client)
- 📦 [Workspace & examples](https://github.com/jscott3201/rusty-modbus)

## Retry and deadline behavior

`ClientConfig::timeout` bounds each request attempt after semaphore admission.
Waiting for admission is not timed; the bounded logical-request envelope starts
when a permit is acquired. The client retries response timeouts and transport
timeouts only for replay-safe reads. Typed writes are not replayed after an
ambiguous timeout or transport failure; either error may occur after the server
applied the write. A configured Server Device Busy (`0x06`) response remains
retryable for reads and writes.

Acknowledge (`0x05`) is terminal. It is returned as
`ClientError::Exception(ExceptionResponse)` to show that the server accepted the
request and is still processing it. The application owns any completion check;
the client does not report Acknowledge as success or replay the request.

## Client lifecycle

`shutdown().await` seals admission before draining. Operations admitted before
the seal keep the reader and deadline scheduler available and retain their retry
policy. A caller already waiting for admission receives
`ClientError::ShuttingDown`; a new call after the seal receives
`ClientError::NotConnected`. When `ClientConfig::shutdown_timeout` expires, the
client cancels remaining operations with `ShuttingDown`, stops the reader and
deadline scheduler, and joins both tasks before returning. Concurrent shutdown
callers wait for the same coordinator, so dropping one shutdown future does not
cancel shutdown.

`abort()` seals admission and requests cancellation immediately. It is
synchronous, idempotent, and can be called without a running Tokio runtime. It
does not wait for task termination; call `shutdown().await` later when a join is
required. Dropping the final client owner uses this immediate path. If a client
is stored in an `Arc`, dropping a non-final handle leaves the shared client
running.

Shutdown completes the client-owned logical lifecycle; it does not guarantee a
transport flush or physical close. `TransportSink` has no close method, and the
client retains the generic sink until the client itself is dropped. Cancellation
can race a transport send after some or all request bytes were accepted, so
mutating requests retain the same ambiguous-write warning as transport errors
and timeouts.

Device Identification pagination admits each page separately. A shutdown seal
between pages rejects the next page instead of treating the full page chain as
one admitted operation.

## License

Licensed under the [MIT license](LICENSE). MSRV: Rust 1.95.
