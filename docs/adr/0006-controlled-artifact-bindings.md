# ADR 0006: Explicit manifests opt variance-run artifacts into content verification

Status: accepted

## Context and owner decision

Controlled-evidence schema v1 validates document structure, references and
canonical integrity identities without opening opaque evidence locators. ADR
0005 separately defined whole-artifact content fingerprints, without assigning
that algorithm retroactively to every controlled-evidence v1 digest.

The owner chose **Binding manifest (Recommended)** over a new contract version.
An additive, explicitly supplied manifest can pin one exact canonical contract
and opt its variance-run artifact references into the already selected
whole-artifact scheme. It avoids silently changing existing v1 consumers or
requiring a contract-version migration for this bounded verification step.

## Decision

Add `verify-controlled-artifacts <contract_json> <bindings_json>` and the
corresponding `verify_controlled_artifacts` module function. The separate
`benchmark-controlled-artifact-bindings` v1 manifest contains exactly:

- its binding schema name/version;
- `contract_sha256`, the whole canonical contract SHA-256, including its approval
  record if present, not a raw-file or approval-scope hash;
- the explicit `benchmark-artifact-content` v1 scheme;
- mappings from every variance-run `artifact_evidence_id` to one explicit local
  repository-relative artifact directory.

The mapping set must equal all variance-study run references exactly. Additional
retention records cannot be selected by this command. Schema, canonical pin,
reference coverage, and all directory paths are checked before artifact
fingerprinting. Duplicate IDs, paths and local directory object aliases fail
closed. Input files are limited to 1 MiB and 64 nested containers; the manifest
is capped at 128 mappings without changing controlled-evidence v1 acceptance.

Bindings normalize by evidence ID. The manifest hash uses its entire canonical
compact sorted-key JSON, UTF-8 without ASCII escaping, and one final LF. The exact
schema, synthetic example, encoding and result fields are specified in
[benchmarks.md](../../benchmarks.md#explicit-controlled-artifact-bindings).

## Verification boundary

For each mapping, in evidence-ID order, invoke the existing whole-artifact
fingerprint function. It revalidates raw retained files, strict checksums and
intrinsic report completeness using local-only Git objects. Require the fresh
digest to match the declared retention digest and the rebuilt source to match
`bench-full`, target SHA and run ID. Reject duplicate actual content/identities.
Do not use copied reports, supplied fingerprint JSON, evidence locators, or a
second baseline-selection mechanism as substitutes.

Only mapped variance-run `benchmark_artifact` references receive this explicit
whole-artifact digest interpretation during this command. Existing schema-v1
validation and canonical/approval-scope hashes, the fingerprint contract,
artifact producers, and other consumers retain their behavior. Neither the
manifest nor the verifier reinterprets every `evidence_retention[].sha256`.

Verification is sequential and releases full inventories between artifacts.
It emits one versioned `benchmark-controlled-artifact-verification` v1 result
only after all mappings match. A later failure produces no partial JSON result.
The result pins the contract and manifest and summarizes matched content/run
identities; it does not copy the contract's baseline/approval states as verdicts.

## Qualifications and consequences

- Exit 0 means bound artifact content/run identities matched, not performance
  pass, entire-contract verification, approval, or baseline acceptance.
- Output always states the limited verification scope, integrity-only/no-owner-
  authorization qualification, fixed `not_eligible` performance-enforcement
  state, and explicit `not_verified` categories.
- Producer-set, scenario-set and budget scenario-identity digest comparisons
  remain undefined externally and unverified. Intrinsic per-artifact report
  checks do not supply those external byte definitions.
- Runner/profile control, environment equality, statistical method execution,
  variance analysis, independent executions, other retained evidence contents,
  expiration/continued retention, authentication, approval and enforcement are
  not established. Timestamp structure remains validated without a new clock-
  based retention decision.
- A canonical pin is not a signature or owner authorization. Opaque locators
  are never fetched. There is no network/Cargo/benchmark work, collection,
  persistent write, implicit latest selection, workflow or policy activation.
- Documents/artifacts must remain unchanged throughout the operation. Static
  local checks do not provide an atomic snapshot or race-proof filesystem
  sandbox; unavailable local Git objects fail closed without fetching.
- The disabled policy, existing `controlled-evaluate` exit-3 behavior, and ledger
  evidence status are unchanged. There is no implicit migration, and the broader
  PR-601 acceptance work remains unfinished.
