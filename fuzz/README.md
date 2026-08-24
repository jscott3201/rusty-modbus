# Codec, frame, and RTU assembler fuzzing

This directory is an isolated Cargo workspace for bounded parser and event targets.
Its lockfile and target directory are separate from the repository workspace;
`rusty-modbus-codec` remains a no-default-features dependency. The `frame`
feature selects codec targets that use `rusty-modbus-frame`; the `assembler`
feature selects the RTU timing target without enabling physical serial support.

## Toolchain

The checked-in pins are:

- `nightly-2026-08-15`, with `rust-src` and `rustfmt`;
- `cargo-fuzz 0.13.2`;
- the dependency versions in `fuzz/Cargo.lock`.

Install the tools without replacing the repository's stable 1.95.0 toolchain:

```console
rustup toolchain install nightly-2026-08-15 --profile minimal \
  --component rust-src --component rustfmt
cargo install cargo-fuzz --version 0.13.2 --locked
```

`python3 scripts/fuzz.py check` verifies those pins, the isolated locked Cargo
metadata, and every retained corpus hash before a run.

## Target contracts

| Target | Boundary | Contract |
|---|---|---|
| `pdu_decode` | `decode_pdu_ref`, `decode_request`, `decode_response` | Calls all public dispatchers on at most 254 bytes. Empty, malformed, unknown, and oversized inputs are expected errors. File Record and Device Identification are reached through retained inputs. |
| `mbap_stream` | `MbapCodec::decode` | Appends bounded chunks, drops each decoded frame, and requires every emitted frame to reduce the retained buffer. A decoder error ends that input. Decoded frames are encoded and decoded once for consistency. |
| `rtu_assembler` | `RtuFrameAssembler` byte and deadline events | Runs at most 512 four-byte events with explicit timestamps. It covers exact t1.5/t3.5 boundaries, quarantine, stale and early deadlines, timestamp errors, overflow, whole-candidate CRC, and fixed ADU bounds. Emitted ADUs must contain 4 through 256 bytes and pass CRC validation. |
| `rtu_frame` | `RtuCodec::decode` | Treats at most 257 bytes as one complete candidate ADU. It checks CRC/frame behavior only; it does not model serial reads, t1.5, or t3.5. |
| `rtu_tcp_stream` | `RtuOverTcpCodec::decode` | Uses the incremental bounds from `mbap_stream` while preserving the current first-valid-CRC-prefix and exact-256-byte CRC-miss behavior. It does not define a stricter extension boundary policy. |

The two stream targets interpret their input prefix as a chunk schedule. The
first byte selects one through sixteen schedule bytes. Each schedule byte maps
to an append width of one through 64 bytes; the remaining bytes are the stream.
Target input is capped at 2,048 bytes. After `Ok(None)` the target waits for the
next append, and after `Err` it discards the decoder.

The assembler target uses a ten-byte header for validated nanosecond timing and
an initial epoch, followed by four-byte operations. Timestamp modes select exact
boundaries, nearby values, regressions, and values near `u64::MAX`; deadline
operations choose the active token or a retained stale token. The harness keeps
only one stale token and caps processing at 512 events.

These harnesses bound input copies, decoder calls per append, retained buffers,
and encoded round-trip buffers. They do not retain decoded frame sequences.

## Replay and campaigns

Replay always passes a sorted, nonempty list of individual retained files to
libFuzzer. It never passes a corpus directory. Each file runs once in
libFuzzer's default single process with the recorded target seed, a two-second
per-input timeout, a 2 GiB RSS limit, a 2,048-byte input limit, and final
statistics:

```console
python3 scripts/fuzz.py replay
python3 scripts/fuzz.py replay pdu_decode mbap_stream
```

A campaign accepts one target, positive duration, and explicit seed. Scheduled
CI derives a nonzero 32-bit seed from the workflow run ID and a target-specific
offset, then records it in the run metadata. The script copies the target's
retained inputs into a temporary corpus, so libFuzzer does not add files to the
reviewed corpus:

```console
python3 scripts/fuzz.py campaign rtu_assembler \
  --seconds 60 --seed 3230003003
```

Generated logs, metadata, and crash artifacts are written below
`fuzz/artifacts/` unless `--output` selects another directory. Metadata records
the commit, command, pins, seed, requirement-ID union, and artifact paths.
An output path must be new or carry the ownership marker from an earlier run;
the tool rejects unmarked directories, symlinks, and protected repository paths.
The final temporary campaign corpus is copied to `generated-corpus/` in that
output for optional manual review; the committed corpus is not changed.
After a failed run the script attempts a bounded `cargo fuzz tmin` for each new
artifact. A minimization error is recorded but does not replace the original
failure status.

## Retained corpus changes

`fuzz/corpus/manifest.json` is the corpus inventory. A retained input needs a
requirement-target-case filename, requirement IDs already present in the
conformance ledger, provenance, a narrow contract, a class, and its SHA-256.
`python3 scripts/fuzz.py check` rejects missing files, hash drift, noncanonical
ordering, and files absent from the manifest.

Campaign corpus and artifacts are never promoted automatically. Promotion is a
reviewed pull-request change to the input and manifest. Keep the input minimal
and state whether it is a valid, malformed, boundary, or regression case.

## Exclusions

The assembler target accepts synthetic trustworthy timestamps; it does not
obtain timestamps from `SerialTransport`, reconstruct timing hidden in reads, or
verify OS/USB chunk behavior. This package also excludes transport recovery
policy, clients, servers, network I/O, gateways, TLS, and Python bindings.
RTU-over-TCP boundary/recovery policy remains assigned to PR-104. Fuzz execution
is internal repository evidence; it does not establish physical interoperability
or formal conformance status.
