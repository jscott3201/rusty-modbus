# ADR 0007: Versioned producer and scenario sets follow comparison identities

Status: accepted

## Context and basis

ADR 0005 selected whole-artifact content identity. ADR 0006 selected separate,
explicitly pinned binding manifests rather than changing controlled-evidence
contract v1. Binding v1 verifies content/run identity while leaving producer-set
and scenario-set digest declarations unverified.

This slice carries those opt-in decisions forward. The repository's comparison
model already requires complete producer records and complete scenario workload
keys. Those existing inputs, not measured values or runner metadata, are the
basis for two explicitly versioned set-identity schemes. No control provider,
statistical method, budget, authority or retention decision is introduced.

## Decision

Define two domain-separated preimages:

- `benchmark-producer-set` v1: `set_schema` and all complete producer records in
  `producers`, sorted by UTF-8 `id` bytes. Each record has exactly `adapter`, `id`,
  `producer`, and `version`, all non-empty strings. Duplicate IDs fail.
- `benchmark-scenario-set` v1: `set_schema` and all scenario projections in
  `scenarios`, sorted by each complete canonical projection's UTF-8 JSON bytes,
  including its final LF. Each projection has exactly `kind`, `producer_id`, and
  `identity`, validated using the existing comparison identity rules. Duplicate
  complete identities fail, even if source paths differ.

TCP workload identity includes clients, duration seconds, in-flight depth,
operation, registers, repetitions, transport and warmup seconds. Criterion
identity includes benchmark ID. Both include kind and producer ID. Exact strings
and integer types are retained; booleans, floating-point or string substitutes
for integers are rejected. Pure helper normalization does not relax the existing
report validator's supported producer records or prescribed order.

SHA-256 hashes compact sorted-key JSON using comma/colon separators, standard
JSON string escaping, direct non-ASCII Unicode without normalization, no
non-finite values, UTF-8 without BOM, and one final LF. Each schema name/version
is inside its preimage. Precise examples and independently checked vectors are
in [benchmarks.md](../../benchmarks.md#producer-and-scenario-identities-with-opt-in-binding-v2).

Metrics, correctness counters, samples, retained-evidence locations, run ID/SHA/mode,
timestamps and runner/environment metadata are outside both preimages. Producer
labels (including script-path strings) and Criterion benchmark IDs remain identity
data. Workload inputs such
as repetitions, duration and warmup are inside. Per-projection bytes establish
ordering only: this decision does not define a budget scenario-identity digest.

## Artifact admission and manifest v2

Add read-only `artifact-identities <run_dir>` for supported retained smoke/full
artifacts. It uses the same strict tree/checksum/report admission as fingerprint
derivation, including local-only target-SHA Git evidence. It never accepts a
copied report or identity JSON as proof of retained-artifact validity.

A small private evidence-loading seam returns the existing fingerprint plus
the rebuilt report. Public `fingerprint_artifact` output and binding-v1 call
behavior remain unchanged. New identity derivation and v2 consume the validated
report without a second full artifact-validation pass. Full inventories/reports
are released between sequential v2 bindings.

Binding manifest v2 has the original four fields plus required, explicit
`producer_set_schema` and `scenario_set_schema` records selecting these v1
schemes. Mixed/missing fields and unsupported or boolean versions fail before
artifact work. Each fresh digest must match both its variance-run and study
declarations. Whole-content, canonical contract pin, complete mapping coverage,
directory alias, full-mode and source-identity checks remain mandatory.

V2 results use verification version 2, include compact matched digests and scheme
identities, and remove only the two newly checked fields from `not_verified`.
The fixed `not_eligible` state remains. Binding v1 canonical bytes/hashes, result
shape/bytes and qualifications stay unchanged. There is no implicit upgrade or
change to controlled-evidence contract v1 or its approval-scope hashing.

## Consequences and limits

- Matching sets identifies producer records and workload inputs, not measured
  equality, whole-artifact equality, report-comparison eligibility, runner
  control, independent executions, statistical significance or performance.
- Unsupported source producers still fail existing artifact/report validation;
  pure encoding helpers alone attest neither support nor execution.
- Budget identity, runner/environment equality, statistics/variance, independence,
  other evidence, continued retention, authentication, approval, baseline
  acceptance and enforcement remain unverified. No fields confer owner authority.
- Owners may explicitly choose contract inputs using derived hashes, but must
  separately handle any approval-scope structure and canonical pin changes.
  The commands do not rewrite or approve contracts. Hashing is not authorization.
- Existing path/input limits, streaming hashes and no-fetch Git rules apply.
  Inputs must remain unchanged; no atomic or race-proof snapshot is claimed.
  There is no collection, network/Cargo/benchmark execution, persistence,
  implicit latest selection, policy activation or clock-based decision.
- ADRs 0005/0006 retain their historical v1 meaning. Broader PR-601 acceptance
  remains unfinished; no ledger evidence is promoted by these identities.
