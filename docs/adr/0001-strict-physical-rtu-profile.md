# ADR 0001: Strict physical RTU configuration is additive

Status: accepted

## Decision

Physical serial callers that need a Modbus character format use
`StrictRtuConfig` and `SerialTransport::open_strict`. `RtuConfig` and
`SerialTransport::open` remain the compatibility path with the existing
9600/8N1 defaults and floating-point timing behavior.

`StrictRtuConfig` stores a nonzero baud rate, one of 8E1, 8O1, or 8N2, and the
response timeout. Resolving it produces one immutable snapshot containing the
serial driver settings and timers. `open_strict` maps that snapshot to
`tokio-serial`, uses its t3.5 for transmit spacing, and retains it in both
transport halves for diagnostics.

At rates through 19,200 bit/s, character time, t1.5, and t3.5 are each computed
directly from the baud rate with integer nanosecond ceiling division. t1.5 and
t3.5 do not inherit rounding from character time. Above 19,200 bit/s, the
snapshot labels the fixed 750 microsecond and 1.750 millisecond timers as the
serial-line guide's recommendation.

The strict physical transport also classifies Unit Identifiers by direction.
Client destinations accept 0 through 247; responder sources accept 1 through
247. Errors remain typed as the source of `TransportError::Io`, using
`InvalidInput` for sends and `InvalidData` for receives.

## Why the compatibility path remains

`RtuConfig` has public fields and a 9600/8N1 default. Changing those values or
making `SerialTransport::open` reject configurations would break existing
callers. Conversion with `StrictRtuConfig::try_from(&legacy_config)` gives those
callers an explicit migration check without changing the old API.

The facade mirrors this boundary. `rtu` keeps RTU-over-TCP and configuration
support without `tokio-serial`; `rtu-serial` adds the physical dependency and is
included by `full`.

## Boundaries left in place

This decision does not add receive-side t1.5 assembly, operating-system chunk
independence, wire-drain or RS-485 direction control, expected-peer response
correlation, or broadcast operation policy. The gateway still uses RTU over TCP,
and there is no first-party physical RTU responder. The strict path narrows
configuration and address classes; it is not an interoperability or formal
certification claim.

The timing arithmetic runs once when configuration is resolved and is O(1). It
is not performance-sensitive.
