# ADR 0002: RTU receive assembly requires trustworthy byte timestamps

Status: accepted

## Decision

`RtuFrameAssembler` is an event-driven core with no clock, runtime, or I/O
adapter. A caller submits each byte with a monotonic nanosecond timestamp and
submits tokenized t3.5 deadlines returned by the assembler. Timing is validated
once as `0 < t1.5 < t3.5`; `ResolvedRtuConfig` is the physical-profile source for
those intervals.

The core retains one `[u8; MAX_RTU_ADU_SIZE]` candidate. A byte after t1.5 and
before t3.5 discards that candidate and starts quarantine without retaining the
violating byte. A t3.5 boundary closes the complete candidate, checks its ADU
length and whole-buffer CRC, and may retain the boundary byte as the start of the
next candidate. Corrupt timing and overlength input remain quarantined until
t3.5 has elapsed since the latest observed noise.

Obsolete deadline tokens are stale before timestamp ordering is checked. This
makes delayed callbacks harmless and gives equivalent assembly state when a byte
and its active deadline are processed in either order at the exact boundary.
Errors other than diagnostic increments are transactional.

## Why timestamps are an input

RTU t1.5 invalidation depends on the gap between bytes on the wire. An
`AsyncRead` completion time identifies when a buffer became available, not when
each byte arrived. Splitting one buffered read into synthetic intervals would
invent timing and make framing depend on driver, USB, and scheduler behavior.

The assembler therefore accepts timing only from an adapter that can state a
trustworthy per-byte contract. No such adapter is part of this decision.
`SerialTransport` remains `SerialStream -> Framed<RtuCodec>` and is unchanged.

## Evidence boundary

Unit, conformance, retained-corpus, and fuzz evidence covers the deterministic
state machine for supplied timestamps. It does not cover timestamp acquisition,
OS read-chunk invariance, USB buffering, bus turnaround, or physical RTU
interoperability. Physical findings F-001 and F-003 remain open until an adapter
and its platform evidence satisfy those missing contracts.
