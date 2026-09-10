# ADR 0009: Export complete recorded run metrics after v3 verification

Status: accepted

## Context and bounded decision

Binding v3 already verifies all variance-run artifacts and matches every budget
target in each explicitly selected study/run. Its results intentionally contain
no measured values. The next locally verifiable step is an additive recorded-data
export, not a statistical-method implementation or a new binding version.

Add `study-observations <contract_json> <bindings_json>` and the corresponding
module function. Require an explicit v3 manifest; v1/v2 fail before artifact work
rather than being upgraded. Existing contract, manifest, verification and
derivation schemas/results retain their meaning and canonical bytes.

## Same-pass verified capture

Use one private verification orchestration path with an explicit capture mode.
The ordinary public verifier keeps its signature, result and call paths. Capture
mode adds temporary metric views to the same individual scenario index used for
matching, then deep-copies the complete selected metric into each observation.
It does not load a saved report/verification result, reread artifacts in a second
pass or create a weaker verification path.

All existing v3 gates remain mandatory for all artifacts, including unbudgeted
studies. A late mismatch yields no partial JSON. Full reports, inventories and
metric views are released before the next run; only bounded detached metric
objects and compact verified provenance remain.

## Record unit and provenance

The new `benchmark-study-observations` v1 result embeds the unchanged complete v3
verification result and emits one row per
`(budget_rule_id, study_id, artifact_evidence_id)`, sorted by that tuple. Each row
identifies its declared budget/scenario label, verified study/run/artifact,
whole-content and individual-scenario digests, declared metric tuple and actual
report unit. The embedded verification supplies canonical contract/manifest pins,
all-artifact coverage and per-budget effective study scope/counts.

The observation unit is `retained_bench-full_run`, not a proven independent
execution. Multiple budgets may produce multiple rows for one artifact; every
row retains the same actual artifact identity/digest so it cannot legitimately be
counted as a new execution. No duplicate or missing row keys are accepted.

## Metric objects, not chosen scalar estimators

Copy the entire matched report metric object, preserving every key, integer,
finite number, null CV and supported signed-zero value. Preserve original report
units: throughput `operations_per_second`, p99 `milliseconds`, and Criterion
`nanoseconds`. Separate budget-unit spellings do not cause numeric conversion.

TCP objects retain `recorded_statistics` with count, min, max, mean, median,
sample standard deviation and coefficient of variation. Count is repetitions
within one retained run, not independent benchmark runs. P99 statistics describe
recorded per-repetition p99s, not a pooled latency distribution or global p99.

Criterion objects retain confidence level, lower/upper bounds, point, standard
error and unit. These are the producer's within-run estimate/interval, not
cross-run uncertainty. No mean/median/point is selected as a scalar observation,
and no confidence intervals are combined.

Existing report construction still performs its established within-run
recomputation/coherence validation. Export adds no further metric aggregation,
effects, uncertainty, variance inference, estimator choice or limit comparison.
Opaque contract method IDs are not dispatched to executable methods. Future
inferential-method choices remain explicitly deferred.

## Bounds and qualifications

- Enforce an export-only inclusive cap of 4,096 rows, calculated from validated
  scopes before artifact payload hashing/Git/report work. Reject excess rather
  than truncating. Ordinary v3 verification is not subject to this cap.
- Emit fixed `cross_run_analysis: not_performed` with reason
  `recorded_run_metric_export_only`. Preserve v3 `budget_evaluation: not_evaluated`,
  `performance_enforcement: not_eligible` and its full `not_verified` list.
- Matching/export does not establish authentication, producer execution,
  runner control, independence, statistics, continued retention, approval,
  baseline acceptance, threshold outcomes or enforcement eligibility.
- No input writes, collection, network/fetch, Cargo/benchmark execution, policy
  calls, implicit latest selection or clock-based decision are introduced.
  Local-only Git rules and existing parser/resource limits remain in force.
- Inputs must remain unchanged throughout the call; no atomic or race-proof
  snapshot is claimed. Prior ADRs remain historical and unchanged.
- Broader PR-601 acceptance remains unfinished, with no ledger evidence promotion
  or publication of real approval/production datasets.

Exact root/row fields, metric semantics, command usage, bounds and exit behavior
are documented in [benchmarks.md](../../benchmarks.md#verified-study-observation-export).
