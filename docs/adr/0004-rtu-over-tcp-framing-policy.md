# ADR 0004: RTU-over-TCP framing is an explicit bounded extension policy

Status: accepted

## Decision

RTU-over-TCP remains a project extension that carries RTU ADUs on a TCP byte
stream. It is neither Modbus/TCP (there is no MBAP header) nor physical RTU
(there is no t1.5/t3.5 boundary). No interoperability or certification status
is inferred from this policy.

`RtuOverTcpCodec` keeps the historical first-valid-CRC-prefix scanner as the
named `CrcScanCompatibility` default. `FunctionAwareStrict` is opt-in and
requires the caller to state whether incoming frames are requests or responses.
It derives one legal boundary from the supported standard function grammar,
checks the CRC only at that boundary, and leaves coalesced bytes buffered.
Encoding does not depend on policy or direction.

Both policies make a terminal error decision when malformed input reaches the
256-byte RTU ADU bound. Strict mode also terminates on an unsupported or
indeterminate grammar, an overflowing declared shape, or a bad CRC at the
derived boundary. The framed connection must be discarded after any decoder
error; the codec does not discard bytes, scan for a later boundary, or define
resynchronization.

Strict framing derives lengths for the fixed, byte-count, word-count, exception,
and MEI 0x0E forms listed in the RTU-over-TCP API documentation. Length derivation
does not validate quantities, values, control fields, or other PDU semantics.

## Unsupported strict forms

Strict mode rejects custom, user-defined, reserved, and unknown function codes;
exception-marked requests; MEI types other than 0x0E; and malformed or
overflowing MEI 0x0E object sequences. FC 0x08 Return Query Data remains
indeterminate because its data is variable, and Force Listen Only Mode is
indeterminate because it has no normal response. Unknown diagnostic
sub-functions are also rejected. Strict mode does not use request correlation to
derive vendor functions, FC 0x08 query lengths, or any other response length.

Compatibility mode continues to accept these forms by scanning for the first
CRC-valid prefix. That intentional residual ambiguity is why compatibility is
not presented as the stronger boundary policy.

## Migration

Existing bare/default codec construction and
`RtuOverTcpTransport::connect` retain compatibility framing.
`RtuOverTcpTransport::connect_with_framing_policy` opts a client connection into
strict incoming response framing.

Gateway routes now carry a per-route policy. Because `RouteEntry` was publicly
constructed with struct literals, adding the field is a pre-1.0 source migration.
Use `RouteEntry::new(range, address)` for compatibility framing and chain
`with_rtu_over_tcp_framing_policy(FunctionAwareStrict)` to opt in. Broadcast
forwarding remains send-only, and encoding is unchanged.

## Consequences

Supported strict forms cannot be shortened by an accidental CRC-valid prefix,
and exact-bound corruption cannot wait indefinitely for byte 257. Compatibility
deployments retain their historical valid-wire behavior and residual
first-prefix ambiguity. Applications needing unsupported grammars must stay on
compatibility framing or provide a separately reviewed framing design; this
decision does not add request correlation or a recovery protocol.
