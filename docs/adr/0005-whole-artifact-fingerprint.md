# ADR 0005: Fingerprint the whole retained benchmark artifact

Status: accepted

## Context and owner decision

Benchmark artifacts retain raw evidence, command records, metadata, summaries,
and sometimes derived reports. Existing checksum verification detects corruption
but can be defeated by rewriting both files and the checksum inventory. A
deterministic content identifier is useful for explicitly naming retained bytes;
it is not an authentication or approval mechanism.

The owner explicitly selected **Whole artifact (Recommended)** rather than
**Report only**. The latter would identify only a canonical rebuilt report and
could leave raw evidence, stored reports, or other retained bytes outside the
identity. It is rejected for this slice, even though rebuilding the report
remains necessary to validate semantic completeness.

## Decision

Add the read-only `fingerprint-artifact <run_dir>` command for one explicit,
repository-relative, complete `bench-smoke` or `bench-full` artifact. Its
`benchmark-artifact-fingerprint` v1 result contains a digest, the exact preimage
object, rebuilt run identity, and a fixed integrity-only qualification.

The preimage uses the separate `benchmark-artifact-content` v1 schema and a file
inventory. Every retained regular file is included except the **root**
`checksums.sha256`, including raw evidence and any derived reports. Each entry
contains its run-relative POSIX path and lowercase SHA-256 of its actual raw
bytes. Entries are sorted by UTF-8 path bytes. Canonical JSON sorts object keys,
uses compact comma/colon separators, emits non-ASCII Unicode without escaping
or normalization, uses standard JSON string escaping, rejects non-finite values,
and ends with exactly one LF. SHA-256 of those UTF-8 bytes is the content digest.
The schema name/version is inside the hashed preimage for domain separation.
The digest-bearing result and root checksum file are not recursively hashed.

The precise field/byte contract and independent encoding test vector are in
[benchmarks.md](../../benchmarks.md#whole-artifact-content-fingerprint).

## Validation and effects

Before legacy validators open payloads, a new-command-only preflight rejects
symlinks, nonregular entries, unsafe or ambiguous checksum paths, incorrect
ordering, duplicates, and incomplete inventories. It bounds checksum input to
4 MiB and the tree to 10,000 descendant entries. Nested files named
`checksums.sha256` are explicitly rejected: the legacy writer/validator skips
that basename at any depth, and this command must not silently leave those
bytes out. No artifact producer or existing checksum behavior changes.

The existing report builder then verifies the artifact and reconstructs its
semantics from source evidence, including target-SHA producer and scenario
requirements. A copied report is not sufficient. Older supported artifacts
without stored reports remain valid inputs. Read-only Git object queries are
permitted, with lazy fetching and replacement objects disabled for this command;
missing local objects fail closed. There is no network fetch, Cargo invocation,
benchmark measurement, implicit discovery, contract lookup, or persistent write.
Fingerprint hashing streams raw files without a small total-payload-byte cap;
existing semantic report parsing retains its own behavior.

## Compatibility and consequences

- This is a **new** versioned fingerprint contract. It does not reinterpret or
  verify `evidence_retention[].sha256` in the existing controlled-evidence v1
  schema. Any later binding requires its own explicit contract decision.
- Existing commands, artifact finalization, policy state, and ledger evidence
  status are unchanged. The broader controlled-performance acceptance work is
  still unfinished.
- File content, added/removed files, renames, and stored-report changes affect
  identity. Empty directories, permissions, and filesystem timestamps do not.
  The checksum file is verified but excluded, so accepted checksum line-ending
  changes alone do not affect identity.
- The absolute checkout root is not added to inventory paths. Identical retained
  names and bytes yield identical content identity across checkouts. Embedded
  locations and timestamps are still hashed verbatim; rewriting them changes
  identity. This is not a metadata-normalizing relocation scheme.
- Artifacts must remain unchanged during the operation. Static local checks do
  not provide a race-proof filesystem sandbox or an atomic snapshot.
- A digest identifies content, not authenticity, runner control, approval,
  baseline acceptance, independent executions, statistical significance,
  performance quality, or eligibility for policy activation. No controlled
  provider, budget, authority, or retention policy is selected by this decision.
