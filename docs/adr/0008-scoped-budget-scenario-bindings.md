# ADR 0008: Budget scenario matching uses explicit study scopes

Status: accepted

## Owner decision and context

The owner chose **Explicit studies (Recommended)** instead of automatically
applying every budget to every study/run. Historical studies may retain different
workload sets. A rule must match in every run of each explicitly selected study,
not somewhere in a union, while every variance-run artifact remains subject to
the complete v2 artifact verification gates.

This extends the whole-artifact and opt-in versioned-manifest decisions in ADRs
0005–0007. It does not select an approved/latest baseline or introduce budget
evaluation, statistical methods, authority, retention policy or activation.

## Individual scenario identity

Define `benchmark-scenario-identity` v1 with the preimage
`{identity_schema: {name, version}, scenario: {kind, producer_id, identity}}`.
The complete projection uses the existing comparison identity rules and the same
detached-copy safeguards as scenario-set identity. All eight TCP workload fields
or the Criterion benchmark ID are retained with exact string/integer types.

SHA-256 hashes compact sorted-key JSON, UTF-8 without BOM or ASCII escaping,
standard string escaping, no non-finite values, and exactly one final LF. There
is no Unicode normalization. The new envelope distinguishes this identity from
projection sorting bytes and singleton scenario-set digests. Existing set
identities do not change. Metrics, measurements, source locations, run/environment
metadata and the contract's descriptive `scenario_id` label are excluded.

Add read-only `artifact-scenarios <run_dir>` using the shared retained-artifact
admission path. It rebuilds source evidence and emits individual preimages/digests
plus metric descriptors, ordered by individual digest. It never accepts a copied
report/identity file as source proof. Pure `scenario_identity(projection)` encoding
alone does not attest a retained artifact or execution.

## Metric correspondence, not evaluation

The repository's existing pairings and report units define exactly:

- TCP `throughput`, budget `operations_per_second`, `minimum` corresponds to
  report `operations_per_second`.
- TCP `p99_latency`, budget `ms`, `maximum` corresponds to report `milliseconds`.
- Criterion `mean_estimate`, budget `ns`, `maximum` corresponds to report
  `nanoseconds`.

Check the exact scenario kind/producer and the actual rebuilt metric/unit. A
globally valid tuple does not apply to every kind. No numerical conversion or
measurement-versus-limit comparison is performed. Individual identity excludes
the metric; a rule separately selects one supported metric/unit/direction tuple.

## Manifest v3 and coverage

Binding v3 has the six v2 fields plus the explicit individual identity scheme and
`budget_bindings`, each exactly `{budget_rule_id, study_ids}`. Every contract
budget ID must appear once, with a nonempty set of existing study IDs. Duplicate,
unknown, missing, unsupported and extra-selector forms fail closed before any
artifact hashing or Git work. Canonical form sorts budgets by ID and each study
list lexically, rejecting duplicates rather than silently deduplicating.

The caps are 128 budget bindings and 128 study IDs per binding, alongside existing
1 MiB/depth-64/128-artifact limits. The contract's existing global uniqueness of
budget IDs and `(identity digest, metric)` pairs remains unchanged. A scope does
not authorize duplicate contract rules or require every scenario to have a budget.

All artifacts, including unbudgeted studies, retain the v2 content/run/full-mode
and producer/scenario-set checks. Build one individual index per validated run,
reject ambiguous/duplicate identities, and match each selected rule's digest and
metric in that run. Release full reports/inventories/indexes between runs. Return
only compact scope summaries/counts, not a full matrix or observed values. A
later mismatch yields no partial JSON result.

The digest is authoritative. `scenario_id` is a declared descriptive label,
returned as `declared_scenario_id`, not a report-derived selector. Owners may
derive identities, choose rules/scopes and explicitly refresh structural approval
inputs/pins as needed; the commands never rewrite or approve those documents.

## Compatibility and limits

- V1/v2 manifest canonical bytes/hashes/results, `artifact-identities` v1 output,
  old schema meanings and historical ADRs remain unchanged. V3 is explicit opt-in.
- V3 results remove only budget scenario identity from v2's `not_verified` list.
  They state effective studies and matched-run counts, not universal budget
  coverage of the whole contract.
- Both new derivation and v3 matching state `budget_evaluation: not_evaluated`
  with reason `scenario_metric_matching_only`; performance enforcement remains
  `not_eligible`. Limits, including zero/extreme finite values, are not evaluated.
- Matching is integrity/target identification, not authentication, producer
  execution attestation, runner control, independent execution proof, statistics,
  retention, approval, baseline acceptance, enforcement or performance pass.
- Existing strict path/tree/checksum and local-only no-fetch Git rules apply.
  Inputs must remain unchanged; there is no atomic or race-proof snapshot.
  There is no network, collection, Cargo/benchmark execution, persistence,
  implicit latest selection, policy operation or clock-based decision.
- Broader PR-601 acceptance remains unfinished, with no ledger evidence promotion.

Exact schemas, the independent byte vector, metric descriptors, a complete
synthetic manifest, output fields and exits are documented in
[benchmarks.md](../../benchmarks.md#individual-scenarios-and-explicit-study-budget-scopes-v3).
