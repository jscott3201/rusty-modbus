# Rusty Modbus Benchmark Report

Last updated: 2026-09-01

This document records the current local and Docker performance baseline for the
Modbus/TCP client/server path. The focus is single-connection pipelining: one
TCP client connection, multiple concurrent in-flight requests, and a loopback
server backed by the in-memory store.

## Environment

| Item | Value |
|---|---|
| Git commit | `e776964` on `dev` |
| Host | Apple M5 class MacBook Pro, arm64 |
| OS | macOS 26.5.0 / Darwin 25.5.0 / arm64 |
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Cargo | `cargo 1.95.0 (f2d3ce0bd 2026-03-21)` |
| Docker | `Docker version 29.5.2, build 79eb04c` |
| Local build mode | `cargo run --release` |
| Docker image | Alpine 3.22 runtime and distroless static-debian12:nonroot runtime, Rust 1.95.0 Alpine builder |
| Transport | Modbus/TCP over loopback |
| Server | Spawned benchmark server, `InMemoryStore` |
| Workload duration | 1s warmup + 5s measured per row |
| Client shape | 1 client connection, varied in-flight depth |
| Register count | 10 registers per read operation |

## Reproducible baseline harness

`scripts/baseline.py` records and validates correctness commands and benchmark
samples. It uses only the Python standard library. Its three run modes are:

- `correctness`: runs the formatting, ledger, harness, lint, test, feature,
  example, Python binding, supply-chain, and advisory checks as separate
  recorded commands. This mode requires `cargo-nextest`, `cargo-deny`,
  `cargo-audit`, Python 3.14, and `uv` in addition to the pinned Rust toolchain.
- `bench-smoke`: runs one-second TCP `read` and `mixed` loopback samples at
  in-flight depths 1, 8, and 16, followed by the pipelined TCP Criterion
  throughput rows and both `tcp_pool` lifecycle rows.
- `bench-full`: runs five TCP repetitions at depths 1, 2, 4, 8, and 16, then
  discovers and runs all registered `tcp_*` Criterion targets, including
  `tcp_pool`. Stress measurements default to five seconds with a one-second
  warmup. Codec, server-only, TLS, and RTU targets are outside both benchmark
  modes.

The throughput samples measure TCP loopback performance. The pool lifecycle
rows are narrower observations of the exact work described below, not transport
health or protocol-conformance evidence. The recorded commands in `correctness`
provide the correctness evidence. `.github/workflows/baseline.yml` runs on
Ubuntu only: pull requests and pushes run `bench-smoke`, the Monday schedule
runs `bench-full`, and manual runs select any mode.

Run a mode from the repository root and identify the runner:

```bash
python3 scripts/baseline.py correctness --runner-label local-workstation
python3 scripts/baseline.py bench-smoke --runner-label local-workstation
python3 scripts/baseline.py bench-full --runner-label local-workstation
```

### Uncontended TCP pool lifecycle target

Run both lifecycle rows directly with:

```bash
scripts/bench-local.sh tcp-pool --quick --noplot
```

Both `tcp_pool` rows use one benchmark task and borrower, a one-connection
non-priority pool, no capacity wait, no configured priority device, no
pre-connect, replenishment, or probe, and long idle/health intervals. There is
no concurrent pool activity or Modbus request. "Uncontended" does not exclude
OS or Tokio runtime scheduling.

- `tcp_pool/fresh_get_raw_drop` measures one public `pool.get(addr)` that opens
  a fresh loopback TCP connection, black-boxes only its public address, and
  drops the raw lease so it retires. It includes loopback TCP establishment and
  raw-lease drop; it is not pure pool overhead.
- `tcp_pool/reusable_checkout_handoff_shutdown_return` starts with one idle
  lease seeded outside timing. Each iteration checks it out, hands the pristine
  lease directly to a reusable client with a bounded zero-retry configuration,
  gracefully shuts down the client, recovers the transport, and returns it to
  idle. It includes reusable-client construction, child-task lifecycle,
  graceful shutdown, transport recovery, and idle return; it is not pure pool
  overhead.

Each row creates its Tokio runtime, loopback server, and pool outside its timed
loop. The reusable seed handoff/return is also outside timing. Custom batch
timing stops before exact return-outcome and pool-accounting assertions. Pool
shutdown, final accounting assertions, and server stop are cleanup outside
timing.

These rows and their reports are observational only. They define no threshold,
budget, improvement/regression label, comparison verdict, or accepted baseline;
the two intentionally different rows must not be compared with each other.
Results do not establish cross-run or cross-host comparability, liveness,
health, reconnect behavior, fairness, contention behavior, protocol behavior,
or any gateway, TLS, or RTU claim.

Because report comparison requires identical complete scenario-key sets, an
older report without `tcp_pool/*` keys fails closed when compared with a newer
report. Recollect both operands with identical target sets rather than partially
matching them.

Benchmark modes accept bounded `--duration`, `--warmup`, and `--repetitions`
overrides. All run modes accept `--output-root` and `--run-id`. The default
artifact path is:

```text
bench-output/baseline-v1/<full-40-character-SHA>/<run-id>/
├── environment.json
├── provenance.json
├── commands/<sequence>-<label>/{command.json,command.stdout,command.stderr}
├── stress/parsed/*.json                         # benchmark modes
├── criterion/{raw/**,parsed-estimates.json}     # benchmark modes
├── summary.json
├── summary.csv
├── benchmark-report-v1.json                    # successful benchmark modes
├── benchmark-report-v1.md                      # successful benchmark modes
└── checksums.sha256
```

Schema version `1` is defined in `scripts/baseline.py`. JSON files use sorted
keys and a trailing newline; the CSV has fixed columns. Command records contain
the exact argument array, working directory, UTC timing, exit code, and only
explicit environment overrides. Raw command output and Criterion data remain
the source evidence. `summary.json` and `summary.csv` are parsed views, not a
replacement for those files. Commands inherit the runner environment, so the
artifact is not a hermetic-environment record and does not capture arbitrary
inherited variables or secrets.

The harness binds the artifact to the full SHA from `git rev-parse HEAD`. It
refuses tracked changes and non-ignored untracked files, and it never overwrites
a run directory. Ignored files under `bench-output/` and ignored `.DS_Store`
files do not make the worktree dirty. `--allow-dirty` exists for local
diagnosis; the resulting summary has `status: invalid`, and `validate` rejects
it. A failed command or missing/malformed stress or Criterion output makes the
run fail, but the harness still attempts to write the partial summary and
checksums. TCP stress samples must report zero errors, zero error rate, and zero
retry attempts; the TCP benchmark helper configures zero retries.

`checksums.sha256` covers every retained artifact file except itself and stores
repository-relative paths in bytewise order. Verify a copied or retained run
with:

```bash
python3 scripts/baseline.py validate bench-output/baseline-v1/<SHA>/<run-id>
```

The checksums detect missing or corrupt retained files. They are not a signature
or attestation: rewriting files and regenerating `checksums.sha256` defeats that
check.

### Machine-readable benchmark report contract

Successful `bench-smoke` and `bench-full` finalization now writes
`benchmark-report-v1.json` and `benchmark-report-v1.md` before constructing the
checksum inventory. The workflow already uploads the complete run directory, so
no measured value controls whether these informational reports are uploaded.
Correctness artifacts do not contain TCP stress/Criterion scenarios and do not
produce benchmark reports.

The report uses the independent `benchmark-report` schema version `1`. This does
not bump, reinterpret, or make the report files mandatory for baseline artifact
schema version `1`; retained v1 artifacts without reports remain valid. Render a
report from an existing validated benchmark artifact into a new, repository-local
directory with placeholder paths as follows:

```bash
python3 scripts/baseline.py report \
  bench-output/baseline-v1/<SHA>/<run-id> \
  --output-dir <new-report-output-dir>
python3 scripts/baseline.py validate-report \
  <new-report-output-dir>/benchmark-report-v1.json
```

The render command validates the source artifact and its checksum inventory,
does not modify the source, rejects traversal and symlink output paths, and
refuses an existing output directory. Repeated rendering from identical source
bytes is byte-identical. The report preserves source timestamps rather than
generating a render timestamp.

Each report records the full target SHA, run ID, mode, source status, declared
runner label, recorded environment and tool identity, strict zero-error and
zero-retry facts, normalized stress/Criterion values, and source-relative raw
and checksum references. Producer records identify custom stress JSON schema v1
and the exact Criterion 0.5.1 `new/estimates.json` private-layout adapter. The
renderer obtains that version from `Cargo.lock` at the artifact's validated full
target SHA through Git object storage; unavailable, ambiguous, mismatched, or
unsupported lock evidence is rejected rather than inferred from the current
checkout. That Criterion layout is not presented as a stable upstream API.

Report evidence is explicitly `observational_only`: artifact validity may be
`valid`, while performance comparability and runner isolation remain
`not_proven`, and budget and statistical decisions remain `not_evaluated`.
The report schema itself defines no performance budget, threshold, verdict,
accepted baseline, host-isolation policy, or cross-run comparison. The report
renderer does not compute deltas. Checksums remain an integrity inventory, not a
signature or attestation.

### Whole-artifact content fingerprint

The read-only fingerprint command validates one complete retained `bench-smoke`
or `bench-full` artifact, rebuilds its report from source evidence, and emits a
single versioned JSON document. Replace the quoted placeholder path with an
existing retained run; this command does not collect benchmarks:

```bash
python3 scripts/baseline.py fingerprint-artifact --help
python3 scripts/baseline.py fingerprint-artifact 'bench-output/baseline-v1/<SHA>/<run-id>'
```

The directory must be an explicit repository-relative path, anchored to the
repository containing the script rather than the caller's current directory.
Spaces and Unicode are supported. Absolute paths, empty/dot/traversal components,
backslashes, and symlinks in the path or anywhere in the artifact are rejected.
Every descendant must be a regular file or directory: FIFOs, sockets, devices,
and other special entries are rejected **before** checksum/report readers open
payloads. The root `checksums.sha256` must be a regular file. Manifest paths must
be strict repository-relative paths naming exactly the retained files inside
that artifact, without aliases, duplicates, missing files, or unlisted files.
Digests must be lowercase SHA-256 and paths must be UTF-8 bytewise sorted. LF or
CRLF checksum lines and an optional final line ending are accepted; blank or
malformed lines are not. Nested files named `checksums.sha256` are rejected,
not silently omitted by the legacy checksum walker.

The new preflight bounds the checksum inventory to **4 MiB (4,194,304 bytes)**,
reading at most that limit plus one byte, and the artifact to **10,000 descendant
entries**, including files and directories. There is no small total-payload-byte
limit: fingerprint file hashing streams in 1 MiB chunks, including large raw
logs. Existing report parsing retains its established behavior and resource
characteristics; these guards do not replace its parsers.

Semantic validation uses `build_benchmark_report`, not a copied
`benchmark-report-v1.json`. Failed, dirty, correctness-only, partial, unsupported,
or scenario/producer-incomplete artifacts fail closed. Supported older v1
artifacts without stored reports remain supported. Rebuilding requires local
target-SHA `Cargo.lock` Git objects and, for `bench-full`, the target-SHA
`benchmarks/Cargo.toml`. The command permits only read-only local object queries,
using `git --no-lazy-fetch --no-replace-objects cat-file blob ...`: unavailable
objects or unsupported Git flags cause failure, never a fetch. It does not
bootstrap a run, invoke Cargo, run benchmarks, or contact a network.

#### Fingerprint v1 bytes and output

The output has exactly these fields:

- `fingerprint_schema`: `{"name":"benchmark-artifact-fingerprint","version":1}`.
- `content`: the exact digest preimage object, with
  `content_schema = {"name":"benchmark-artifact-content","version":1}` and
  `files = [{"path": <run-relative POSIX path>, "sha256": <raw-file SHA-256>}, ...]`.
- `sha256`: lowercase SHA-256 of the canonical UTF-8 encoding of `content`.
- `source`: exactly `mode`, `run_id`, and `target_sha`, taken from the rebuilt
  validated report, not the current checkout's HEAD.
- `qualification`: the fixed string
  `integrity_only_not_authentication_attestation_approval_baseline_acceptance_independent_run_proof_performance_verdict_or_policy_activation`.

The inventory includes **all retained regular files except the root
`checksums.sha256` itself**, including raw evidence, metadata, summaries, and any
stored derived reports. Each file's bytes are hashed verbatim. Inventory paths
are relative to the named run directory, with `/` separators, no absolute root,
and no Unicode normalization. Entries are sorted by the UTF-8 bytes of `path`.
Empty directories, permissions, and filesystem timestamps are not content.
The validated checksum file is excluded to avoid self-reference; rewriting its
accepted line endings alone does not change identity.

Both the preimage and CLI output use sorted object keys, compact JSON separators
`,` and `:`, no insignificant whitespace or BOM, unescaped non-ASCII Unicode,
standard JSON string escaping (including quotes and control characters), no
NaN/Infinity, and exactly one final LF byte. This is precisely Python's
`json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
allow_nan=False) + "\n"`, encoded as UTF-8. Output encoding is independent of the
terminal's locale. Only the `content` object is hashed, not the result containing
the digest. Its schema name/version domain-separate this identity from a raw
file hash or a report-only hash.

For an encoding test vector only (not a complete benchmark artifact), `a.txt`
containing zero bytes and `é.txt` containing the three bytes `abc` give this exact
preimage line, followed by LF:

```json
{"content_schema":{"name":"benchmark-artifact-content","version":1},"files":[{"path":"a.txt","sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"},{"path":"é.txt","sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]}
```

Its SHA-256 is `d4e09959da94e5d1a8768ad6e8e7c6480c99b74803d8acd518a2a72193825562`.
An independent consumer can verify each file digest, construct the sorted
preimage, and reproduce the outer digest without the baseline script.

Exit **0** emits one canonical JSON document on stdout and nothing on stderr.
Input/validation failure exits **1** with a stderr diagnostic only, no partial
JSON, and no traceback for handled input errors. Argparse usage errors exit
**2**; help exits **0**. Module callers can use
`fingerprint_artifact(repo_root, run_dir)` and serialize its returned document
with `artifact_fingerprint_json_text`; input failures raise `BaselineError`.

The artifact must remain unchanged throughout validation and hashing. These are
static checks, not an atomic snapshot or a race-proof hostile-filesystem
sandbox. Copying the same files with the same run-relative names and bytes to a
different checkout preserves content identity, provided local target objects
are available. Rewriting embedded checkout locations, timestamps, or other
metadata changes those bytes and therefore the digest; relocation with such
rewrites is **not** promised to preserve identity.

This is integrity-only content identity, not authentication, runner attestation,
approval, baseline acceptance, independent-run proof, statistical significance,
or a performance verdict. It reads no controlled contract, resolves no opaque
locator, writes no files, chooses no `latest`, and activates no policy. In
particular, it does **not** retroactively define or verify
`evidence_retention[].sha256` in controlled-evidence schema v1. The owner's
whole-artifact decision and rejected report-only alternative are recorded in
[ADR 0005](docs/adr/0005-whole-artifact-fingerprint.md). This slice does not
complete the broader controlled-performance acceptance work.

### Observed benchmark report deltas

The independent `benchmark-comparison` schema version `1` consumes two complete,
validated `benchmark-report` v1 JSON files. The first operand is positionally
named `baseline` and the second is positionally named `candidate`; neither name
means that a report has been accepted, promoted, or approved. Emit the canonical
comparison JSON to standard output with:

```bash
python3 scripts/baseline.py compare-report \
  <BASELINE-benchmark-report-v1.json> \
  <CANDIDATE-benchmark-report-v1.json>
```

The command uses the same repository-contained, symlink-rejecting report loader
as `validate-report`, including the target-SHA Criterion identity proof. It does
not modify either input or create an output directory. The two reports must have
the exact `benchmark-report` v1 schema identity, identical producer records, and
the same run mode. Their complete scenario-key sets must be equal; missing,
extra, ambiguous, or duplicate keys are rejected rather than partially matched.

TCP stress keys contain `kind`, `producer_id`, transport, operation, in-flight
depth, clients, registers, repetitions, duration seconds, and warmup seconds.
Criterion keys contain `kind`, `producer_id`, and benchmark ID; duplicate
Criterion benchmark IDs are rejected even when their private source paths are
different. Paired metrics must have identical units and shapes, and Criterion
confidence levels must be exactly equal before point estimates are observed.

Each matched TCP scenario records only the two input means and signed
`candidate_minus_baseline` for throughput (`operations_per_second`) and p99
latency (`ms`). Each matched Criterion scenario records only the two mean point
estimates and the same signed subtraction in `ns`. The output preserves each
operand's full target SHA, run ID, mode, declared runner label and recorded
runner context, source artifact provenance, and producer records. It does not
require or infer runner or environment equality.

Comparison evidence remains fixed to `classification=observational_only`,
`performance_comparability=not_proven`, `runner_isolation=not_proven`,
`budget_decision=not_evaluated`, and
`statistical_significance=not_evaluated`. Schema v1 defines no percentage,
direction label, improvement or regression wording, threshold, pass/fail,
budget verdict, confidence inference, statistical test, accepted baseline, or
performance decision. It generates no timestamp. Scenario-key ordering,
sorted-key JSON, and a trailing newline make repeated rendering of identical
inputs byte-identical.

### Disabled benchmark budget policy preflight

The checked-in
`benchmarks/policy/benchmark-budget-policy-v1.json` manifest uses the separate
`benchmark-budget-policy` schema version `1`. The production policy state is
explicitly `disabled`. It contains no numeric threshold, budget rule, approved
baseline, variance evidence, statistical method, controlled runner/profile, or
approval record. It does not name an owner or turn arbitrary text into approval.
Validate it from the repository root with:

```bash
python3 scripts/baseline.py validate-policy \
  benchmarks/policy/benchmark-budget-policy-v1.json
```

Policy loading is strict and fail-closed. The validator requires the exact v1
keys and types, rejects unsupported schemas and states, duplicate or incomplete
blocker sets, non-finite or boolean numeric substitutions, active values in a
disabled policy, and absolute, traversal, or symlink policy paths. Object key
order and activation-blocker order do not change canonical policy identity.
Unsupported future active policy content is rejected rather than partially
evaluated.

`controlled-evaluate` is a read-only preparation command. It requires an
explicit policy and two explicit, distinct `bench-full` artifact directories:

```bash
set +e
python3 scripts/baseline.py controlled-evaluate \
  benchmarks/policy/benchmark-budget-policy-v1.json \
  bench-output/baseline-v1/<BASELINE-SHA>/<BASELINE-RUN-ID> \
  bench-output/baseline-v1/<CANDIDATE-SHA>/<CANDIDATE-RUN-ID> \
  > controlled-evaluation-v1.json
status=$?
set -e
test "$status" -eq 3
```

All three operands must be repository-relative, traversal-free paths inside the
current checkout and must not use symlinks. Both complete artifact directories,
including their checksum inventories, are validated. Reports are rebuilt from
the retained raw artifact sources; copied `benchmark-report-v1.json` files are
not trusted as evaluation inputs. The rebuilt reports must have matching
producers, mode, and exact complete scenario sets. Missing, expired, failed,
dirty, partial, checksum-invalid, duplicate-identity, smoke-mode, or otherwise
incompatible evidence is rejected. There is no implicit `latest` lookup,
download, fallback, partial matching, persistence, artifact mutation, or
baseline promotion.

With the checked-in disabled policy, successful preflight emits canonical
`benchmark-controlled-evaluation` schema version `1` JSON to standard output
and exits `3`. The document records `performance_enforcement.state` as
`not_eligible` with these bounded reason codes:

- `policy_disabled`
- `no_approved_controlled_baseline`
- `no_approved_repeated_variance_evidence_or_statistical_method`
- `no_approved_controlled_runner_or_profile`
- `no_approved_budgets_or_approval_path`

Exit `3` is intentionally nonzero so this result cannot be interpreted as an
enforcing or passing performance gate. Invalid policy or artifact input exits
`1` and emits no evaluation document or performance decision. The embedded
comparison remains the unchanged observational v1 comparison:
`performance_comparability` and runner isolation are `not_proven`, while budget
and statistical decisions remain `not_evaluated`. In particular, a declared
runner label and recorded environment are not a controlled-runner proof, and
the preflight does not claim environment equality.

Checksums provide an integrity inventory only; neither policy validation nor
controlled evaluation authenticates artifacts, attests a runner, establishes
approval authority, or accepts either operand as a baseline. Pull-request/push
`bench-smoke` and scheduled/manual `bench-full` workflow runs remain collection
only and non-enforcing. The workflow has finite artifact retention and performs
no controlled evaluation.

Future activation requires, at minimum, all of the following to be separately
defined and approved before an active schema/state can be implemented: a
controlled baseline and promotion rules; repeated variance evidence and a
statistical method; a controlled runner and control profile; and numeric budgets
plus an explicit approval path. This disabled preflight defines none of those
items and emits no performance pass/fail, improvement, regression, or threshold
verdict.

### Internal controlled evidence contract (structural validation only)

`scripts/baseline.py` also owns the separate, provider-neutral
`benchmark-controlled-evidence-contract` schema version `1`. This internal
document contract has a read-only file loader and structural-validation CLI.
There is still no checked-in production instance, evaluator, workflow call edge,
benchmark run, baseline promotion, budget calculation, or policy activation for
it. It is intentionally not part of `benchmark-budget-policy` state and is not consumed by
`load_policy_file` or `controlled-evaluate`.

The existing in-memory APIs validate a supplied document, render canonical
sorted-key JSON with a trailing newline, and hash those canonical bytes:

- `validate_controlled_evidence_contract(document)`
- `controlled_evidence_contract_json_text(document)`
- `controlled_evidence_contract_sha256(document)`

`load_controlled_evidence_contract_file(repo_root, contract_json)` reads one
explicit local file, delegates validation and normalization to those same APIs,
and returns the validated normalized document. Input or schema errors raise
`BaselineError`; existing in-memory schema v1 semantics remain unchanged.

For a schema-v1 document that you supply at `bench-output/controlled-evidence.json`
(an example path, not a shipped production contract), run:

```bash
python3 scripts/baseline.py validate-controlled-evidence --help
python3 scripts/baseline.py validate-controlled-evidence bench-output/controlled-evidence.json
```

The positional path must name a regular file relative to the repository root
containing the script, **not the caller's current directory**. Absolute paths,
empty paths or components, `.`/`..` components, backslash separators, symlink files
or ancestors inside the repository, missing files, directories, and special files
are rejected. Spaces and Unicode are supported; quote such paths in the shell.
These are static local-input checks, not a race-proof sandbox against concurrent
filesystem replacement.

The new loader reads at most **1 MiB (1,048,576 bytes)** plus one byte to detect
oversize input; the limit includes whitespace. JSON must be UTF-8, have an object
root, and contain at most 64 nested objects/arrays. Invalid encoding or syntax,
duplicate object names at any depth, `NaN`/`Infinity`, overflow exponents, huge
integers outside finite numeric representability, and strings that cannot be
encoded as canonical UTF-8 are rejected. Finite integers retain their exact
values. These resource guards apply only to this loader, not existing commands.

CLI exit/output contract:

- **0**: stdout is `controlled evidence contract structurally valid (validation only)`
  followed by a newline; stderr is empty.
- **1**: input or schema failure; stderr diagnostic only, no success payload or
  traceback for handled bad input. Diagnostics do not echo document values.
- **2**: argparse usage error, such as missing arguments or unknown flags.

Validation reads only the named contract. It does not resolve or fetch opaque
evidence locators, verify retained evidence contents or continued retention,
discover a `latest` contract, invoke Git/Cargo/benchmarks, access the network,
write files, or activate policy. A structurally valid document is not an
attestation, authentication, approval, baseline acceptance, statistical
significance result, or performance verdict.

Schema v1 requires exact keys and bounded identities. Set-like evidence
references, variance studies and runs, baseline records, budget rules, and
retention records are canonicalized by stable identity; duplicate identities
are rejected rather than deduplicated. The contract records all of the following
without choosing repository values:

- a controlled-runner identity, a versioned control-profile identity, and
  retained evidence binding them together; a runner label or environment record
  alone is explicitly not proof of control;
- a named and versioned statistical method plus retained analysis-plan evidence,
  with a complete independent `bench-full` run as the sample unit, at least two
  independent runs, exact complete scenario-set alignment, and only effect and
  uncertainty outputs; v1 has no statistical test, alpha, significance, or
  verdict field;
- repeated-variance studies whose unique run identities share one full target
  SHA, runner/profile/method identity, producer-set digest, and scenario-set
  digest, with every run and analysis linked to retained evidence. Across the
  complete variance-run graph, every run must reference a distinct retained
  `benchmark_artifact` evidence ID and a distinct SHA-256 artifact content
  digest, so aliases or duplicate-content evidence cannot satisfy the minimum
  independent-run count;
- an exact ordered `baseline_lifecycle.promotion_path` of `candidate ->
  variance_collected -> promotion_pending -> approved`, followed only by
  `approved -> superseded`, plus an explicit-ID-only promotion rule. Each
  `baselines[].promotion_chain` must contain exactly the prefix for its declared
  state: no transitions for `candidate`, the variance transition for
  `variance_collected`, the variance and rule transitions for
  `promotion_pending`, and all three transitions for `approved` or `superseded`.
  The first transition binds the baseline run and retained analysis evidence to
  its variance study, the second binds the retained promotion and rule evidence,
  and only the final transition may bind the approval record and retained
  approval evidence. Superseded records additionally require a distinct approved
  successor and retained supersession evidence. Omitted, reordered, skipped, or
  direct candidate-to-approved stages are invalid; there is no implicit `latest`
  selection;
- uniquely identified budget definitions bound to the digest of one exact,
  complete scenario identity, closed metric/unit/direction combinations, and a
  finite non-boolean non-negative limit; no calculation or performance decision
  is represented;
- an explicit versioned approval-authority identity and optional approval
  record. The approval scope digest covers canonical contract inputs with the
  approval record replaced by `null`, avoiding self-reference. Structural
  validation does not authenticate that record or establish owner
  authorization; and
- retained evidence records with unique IDs, closed evidence kinds, opaque
  bounded locators, SHA-256 content identity, and ordered UTC recorded/retained
  timestamps. Every evidence reference must resolve to the expected kind.

SHA-256 values and locators establish content identity and reference structure
only. They are not signatures, runner attestation, proof of environment equality,
approval authority, or approval. Likewise, successful validation does not assert
performance pass/fail, regression/improvement, statistical significance,
baseline acceptance, or policy activation. The schema-only tests use obviously synthetic,
non-authoritative values and perform no network, artifact, or benchmark work.

The checked-in disabled policy and the read-only `controlled-evaluate` exit-`3`
preflight remain unchanged and non-enforcing. Activating any runner/profile,
method, repeated-variance process, baseline promotion, budget, approval
authority, retention process, or performance gate requires a separate
owner-approved PR. This schema and its validator do not advance controlled
performance acceptance or any ledger evidence status.

### Explicit controlled artifact bindings

`verify-controlled-artifacts` adds an **opt-in binding manifest**, not a new
controlled-evidence contract version. It matches local artifact content and
declared run identity for every variance-study run in one canonically pinned
contract. The existing structural-only validator, contract/approval-scope hashes,
fingerprint command, producers, and policy commands are unchanged.

This section specifies the preserved **v1** behavior. The opt-in v2 extension
below additionally checks versioned producer-set and scenario-set identities;
v1 manifests and their canonical hashes/results are not implicitly upgraded.

Supply both files explicitly; the example paths below are not shipped production
instances:

```bash
python3 scripts/baseline.py verify-controlled-artifacts --help
python3 scripts/baseline.py verify-controlled-artifacts \
  inputs/controlled-evidence.json inputs/artifact-bindings.json
```

Both arguments must be repository-relative regular UTF-8 JSON files, anchored to
the repository containing the script, not the caller's current directory. Spaces
and Unicode are supported. Absolute paths, empty/dot/traversal components,
backslashes, symlink files or ancestors, missing files, directories, and special
files are rejected. The same strict repository-relative policy applies to every
mapped `run_dir`, which must be an existing nonsymlink directory.

#### Binding manifest v1

The manifest has exactly four fields. This complete **syntax example is
synthetic and not ready for verification**: replace the zero contract pin,
example target SHA, evidence IDs, and directories with your explicitly selected
contract and retained artifacts. A copied example grants no approval or authority.

```json
{
  "binding_schema": {
    "name": "benchmark-controlled-artifact-bindings",
    "version": 1
  },
  "contract_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "artifact_content_schema": {
    "name": "benchmark-artifact-content",
    "version": 1
  },
  "artifacts": [
    {
      "evidence_id": "synthetic-artifact-a",
      "run_dir": "bench-output/baseline-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/synthetic-run-a"
    },
    {
      "evidence_id": "synthetic-artifact-b",
      "run_dir": "bench-output/baseline-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/synthetic-run-b"
    }
  ]
}
```

- `binding_schema` and `artifact_content_schema` require the exact names and
  integer versions shown; boolean versions, extra keys, and alternative digest
  schemes are rejected.
- `contract_sha256` is a lowercase 64-hex SHA-256 of the **whole canonical
  contract**, as returned by `controlled_evidence_contract_sha256` after the
  existing loader validates it. It is **not** the raw contract-file hash and
  **not** `approval.scope_sha256`; the whole-contract pin includes the approval
  record when present. Equivalent set/list permutations preserve this pin.
- `artifacts` contains 1–128 exact `{evidence_id, run_dir}` objects. IDs use the
  existing lowercase bounded identifier grammar. Duplicate IDs and directory
  strings are errors, not deduplicated entries. Local directory aliases with the
  same filesystem device/inode identity are also rejected before fingerprinting.
- The mapping ID set must equal **all** unique
  `variance_studies[].runs[].artifact_evidence_id` values. Each must resolve to
  retained `benchmark_artifact` evidence. Missing, extra, unknown, non-artifact,
  or unreferenced-retention mappings fail; there is no subset/best-effort mode.
  Baseline records remain governed by the existing contract validator, not a
  second selection mechanism.

Each input file is bounded to **1 MiB including whitespace** and 64 nested
objects/arrays. The binding loader rejects malformed UTF-8/JSON, non-object roots,
duplicate object names at any depth, non-finite/overflowing numbers, unsupported
numeric representations, and strings not representable as canonical UTF-8.
The 128-mapping cap is new-verifier scope only, not a change to accepted
controlled-evidence v1 documents.

The binding manifest's canonical representation sorts `artifacts` by evidence
ID and object keys lexically, preserving path spelling without Unicode or
filesystem normalization. It uses compact JSON separators `,` and `:`,
`ensure_ascii=False`, `allow_nan=False`, UTF-8, and exactly one final LF. Its
canonical SHA-256 hashes the entire normalized manifest, including its schema,
contract pin, content scheme, and mapping paths. This differs from the existing
contract's indented canonical representation. Module helpers in
`scripts/baseline.py` are `canonical_controlled_artifact_bindings`,
`controlled_artifact_bindings_json_text`, `controlled_artifact_bindings_sha256`,
and `load_controlled_artifact_bindings_file`; these perform syntax validation and
normalization, **not** cross-document or artifact verification.

#### Verification and result scope

`verify_controlled_artifacts(repo_root, contract_json, bindings_json)` validates
the entire contract and manifest, checks the canonical pin and complete mapping
coverage, and validates **all** directory paths before any fingerprint, Git
query, or artifact-payload hashing. It then invokes the existing
`fingerprint_artifact` sequentially in evidence-ID order. For each mapping:

1. Revalidate the actual retained files/checksums and rebuild the report through
   the existing fingerprint path; copied reports or fingerprint JSON cannot
   substitute for raw evidence.
2. Require `bench-full` mode and exact target-SHA/run-ID equality with the
   declared variance run. A valid smoke fingerprint is insufficient.
3. Require the fresh whole-artifact content digest to equal that referenced
   retention record's declared `sha256`, with distinct actual identities/content.

The explicit manifest opts **only these mapped variance-run artifact references**
into the `benchmark-artifact-content` v1 whole-artifact interpretation from
[ADR 0005](docs/adr/0005-whole-artifact-fingerprint.md). It does not globally
reinterpret `evidence_retention[].sha256` for existing v1 consumers or other
evidence kinds. All fingerprint guards and local-only Git rules still apply:
4 MiB checksum inventory, 10,000 descendant entries, streamed file hashes, no
lazy fetch, and failure when required local Git objects are unavailable. Full
inventories are discarded between artifacts; the result retains only summaries.

Success emits one compact, sorted-key UTF-8 JSON document ending in LF, even
under an ASCII stdout locale. Its fields are:

- `verification_schema`: `benchmark-controlled-artifact-verification`, version 1.
- `contract`: `contract_id` and whole-contract `canonical_sha256`.
- `binding_manifest`: binding `schema` and canonical manifest `canonical_sha256`.
- `artifact_content_schema`: the explicit `benchmark-artifact-content` v1 scheme.
- `verified_artifacts`: evidence-ID-sorted records containing `evidence_id`,
  `study_id`, `target_sha`, `run_id`, `mode`, and fresh `content_sha256`.
- `verification_scope`: `variance_run_artifact_content_and_declared_run_identity_only`.
- `qualification`: `integrity_only_not_authentication_or_owner_authorization`.
- `performance_enforcement`: fixed `state: not_eligible` with
  `reason: artifact_binding_verification_only`.
- `not_verified`: the fixed list below, regardless of synthetic or real
  approval-state declarations inside the contract.

Explicitly **not verified**:

| Result label | Outside this verifier's scope |
|---|---|
| `producer_set_sha256` | Comparing the opaque declared producer-set digest |
| `scenario_set_sha256` | Comparing the opaque declared scenario-set digest |
| `budget_scenario_identity_sha256` | Comparing a budget's scenario-identity digest |
| `runner_profile_control_and_environment_equality` | Runner/profile control, attestation, or environment equality |
| `statistical_method_and_variance_analysis` | Method execution, variance analysis, or statistical significance |
| `independent_executions` | Proof that artifacts came from independent executions |
| `non_artifact_and_unmapped_retained_evidence` | Contents of other retained evidence, including unmapped records |
| `expiration_and_continued_retention` | Current expiration or continued availability; structural timestamp checks still apply |
| `approval_authentication_and_owner_authorization` | Authenticating approvals, signers, or owner authorization |
| `baseline_acceptance` | Accepting or promoting a baseline |
| `performance_enforcement` | Budgets, performance verdicts, or enforcement eligibility |

The report builder still checks each artifact's **intrinsic** producer/scenario
completeness. That does not define external bytes for the opaque producer-set,
scenario-set, or budget-identity digests, or compare those fields with artifacts.
Unreferenced retention records and non-artifact evidence locators are not opened.

Exit **0** means all bound content/run identities matched, **not performance
pass** or complete-contract verification. Stderr is empty. Input/verification
failure exits **1**, with a stderr diagnostic only and no partial result, even
if an earlier mapping matched. Handled input errors produce no traceback;
generic parsing/schema diagnostics do not echo supplied document values.
Usage errors exit **2**, and help exits **0**. Module input failures raise
`BaselineError`; returned results use `artifact_fingerprint_json_text` for the
same canonical encoding as the CLI.

Documents and artifacts must remain unchanged during a call; there is no atomic
snapshot or race-proof filesystem guarantee. The command performs no writes,
collection, implicit `latest` selection, opaque-locator fetch, network access,
Cargo/benchmark execution, `controlled-evaluate`, workflow operation, or policy
activation. A manifest pin is integrity identity, not a signature or approval.
[ADR 0006](docs/adr/0006-controlled-artifact-bindings.md) records the additive
opt-in decision. Broader PR-601 acceptance remains unfinished, with no ledger
evidence promotion or implicit migration.

### Producer and scenario identities with opt-in binding v2

Derive complete producer-record and scenario-workload identities from a retained
artifact, without collecting benchmarks or trusting a saved report/identity file:

```bash
python3 scripts/baseline.py artifact-identities --help
python3 scripts/baseline.py artifact-identities 'bench-output/baseline-v1/<SHA>/<run-id>'
```

The quoted path is a placeholder for an existing supported `bench-smoke` or
`bench-full` run. The command shares fingerprint admission: an explicit directory
relative to the script's repository root; strict path/tree/checksum validation;
and a rebuilt report using local target-SHA Git objects with no lazy fetching or
replacement objects. Unsupported, malformed, dirty, incomplete, or copy-only
sources fail closed. The existing report validator's prescribed producer
records **and order remain unchanged**. No source report is read before the
tree guards. Missing local objects never trigger a fetch or weaker fallback.

#### Exact set preimages

Two distinct schema names inside the hashed preimages domain-separate these
identities from each other and from whole-artifact content:

- **`benchmark-producer-set` v1:** exactly `set_schema` and `producers`.
  `producers` contains every complete `{adapter, id, producer, version}` record,
  sorted by the UTF-8 bytes of `id`. All four fields are non-empty strings;
  missing/extra fields and duplicate IDs fail rather than being ignored or
  deduplicated. Changing any record field changes identity, or causes the
  retained source to be rejected if its producer is unsupported.
- **`benchmark-scenario-set` v1:** exactly `set_schema` and `scenarios`.
  Every scenario is projected to exactly `{kind, producer_id, identity}`.
  Projections are sorted by the UTF-8 bytes of each complete canonical projection
  JSON (using the encoding below, including its final LF). This is bytewise
  ordering, not numeric tuple ordering. Duplicate complete comparison keys fail,
  including Criterion identities whose source paths differ.

TCP stress projections require `kind=tcp_stress`, the supported stress producer
ID, and all eight identity fields: `clients`, `duration_seconds`, `in_flight`,
`operation`, `registers`, `repetitions`, `transport`, and `warmup_seconds`.
Criterion projections require `kind=criterion_estimate`, the supported Criterion
producer ID, and exactly `benchmark_id`. Validation reuses the existing complete
comparison identity rules: strings, integers and booleans are not interchangeable,
and unsupported kinds, producers or extra identity fields are errors.

Both preimages use compact sorted-key JSON, separators `,` and `:`, non-ASCII
Unicode emitted directly, standard JSON string escaping, no NaN/Infinity, UTF-8
without a BOM, and exactly one final LF. SHA-256 hashes those exact bytes.
Strings retain their exact spelling without Unicode normalization; integers are
not converted through floating point. The schema envelope is
`"set_schema":{"name":"benchmark-producer-set","version":1}` or the analogous
scenario-set name. Canonical encoding is the existing
`artifact_fingerprint_json_text` encoding.

**Excluded from both set preimages:** measured metrics, correctness counters,
samples, retained-evidence locations/references, source run ID/SHA/mode,
timestamps, and runner/environment metadata. Workload changes such as repetitions, duration or
warmup are **not** excluded. Producer labels (including script-path strings) and
Criterion benchmark IDs remain identity data. Source mode and run identity are
checked separately
by bindings. Matching these set identities does not establish whole-file equality,
full report-comparison eligibility, runner control, or performance comparability.
Canonical projection bytes used for ordering do **not** define the still-
unverified budget `scenario_identity.identity_sha256`; no per-scenario digest
contract is introduced here.

Independent encoding vectors (synthetic records, not valid artifact fixtures),
each shown as one exact line followed by LF:

```json
{"producers":[{"adapter":"A","id":"a","producer":"tool","version":"1"},{"adapter":"β","id":"b","producer":"other","version":"2"}],"set_schema":{"name":"benchmark-producer-set","version":1}}
```

Producer digest: `d7a54f4d353a92f0e37f78bb610c46b07c05d6d437479609275f775c225c1f38`.

```json
{"scenarios":[{"identity":{"benchmark_id":"codec/decode"},"kind":"criterion_estimate","producer_id":"criterion-0.5.1-private-estimates-layout"},{"identity":{"clients":1,"duration_seconds":5,"in_flight":8,"operation":"read","registers":10,"repetitions":5,"transport":"tcp","warmup_seconds":1},"kind":"tcp_stress","producer_id":"rusty-modbus-stress-json-v1"}],"set_schema":{"name":"benchmark-scenario-set","version":1}}
```

Scenario digest: `f34c95968055803ee6753dca4f433da4afe879adcf6cd976de0a15f7dcc16fb1`.

The pure module helpers `producer_set_identity(producers)` and
`scenario_set_identity(projections)` return `{preimage, sha256}`. The latter
accepts exact projections, not full scenario records with metrics/sources.
These helpers validate/encode sets only; they do not prove retained-artifact
validity, supported producer execution, or source authenticity.
`artifact_identities(repo_root, run_dir)` provides the guarded artifact path.

#### Derive, choose contract inputs, then verify

1. Derive identities for each explicitly selected retained run with
   `artifact-identities`. Use `fingerprint-artifact` separately when choosing
   the existing whole-artifact content digests. Do not redirect output into the
   source artifact: that would alter its retained inventory.
2. An owner chooses the contract's study/run `producer_set_sha256` and
   `scenario_set_sha256` values and the mapped whole-content digests. All runs
   must still satisfy the existing contract's study consistency requirements.
   If material contract inputs change, an owner must separately handle any
   approval-scope structure and whole-contract pin refresh. Computing a new hash
   does **not** create, renew, authenticate or authorize approval.
3. Explicitly choose a binding manifest **version 2** with the v1 fields plus
   the two required identity schemes. The following complete example is
   synthetic: replace its zero pin, target SHA, evidence IDs and paths with
   explicitly chosen data; it is not a production approval or ready-to-use file.

```json
{
  "binding_schema": {"name": "benchmark-controlled-artifact-bindings", "version": 2},
  "contract_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "artifact_content_schema": {"name": "benchmark-artifact-content", "version": 1},
  "producer_set_schema": {"name": "benchmark-producer-set", "version": 1},
  "scenario_set_schema": {"name": "benchmark-scenario-set", "version": 1},
  "artifacts": [
    {"evidence_id": "synthetic-artifact-a", "run_dir": "bench-output/baseline-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/synthetic-run-a"},
    {"evidence_id": "synthetic-artifact-b", "run_dir": "bench-output/baseline-v1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/synthetic-run-b"}
  ]
}
```

4. Run the same explicit verifier command:

```bash
python3 scripts/baseline.py verify-controlled-artifacts \
  inputs/controlled-evidence.json inputs/artifact-bindings-v2.json
```

V1 still requires exactly its original four fields. V2 requires exactly six.
Mixed fields, omitted/unsupported schemes, boolean/unknown versions, bad pins,
coverage gaps and unsafe paths fail before artifact work. The canonical manifest
digest includes the version and both scheme records. The whole-contract v1
schema, canonical hash and approval-scope hash are unchanged.

For v2, each artifact's fresh set digests must match **both its run and study**
declarations, in addition to every existing content, identity, `bench-full`,
coverage and alias check. Correct set digests cannot substitute for correct
whole-content bytes. A private shared evidence loader returns the fingerprint
and rebuilt report from one validation path; v2 does not perform a second full
artifact-validation pass. Processing is sequential, and full reports, inventories
and set preimages are released before the next binding.

#### Output and remaining limits

`artifact-identities` emits `identity_schema` named `benchmark-artifact-identities`
version 1, `producer_set` and `scenario_set` objects containing the exact
`preimage` and `sha256`, and source `mode`, `run_id`, and `target_sha`. Its fixed
scope is `producer_records_and_complete_scenario_workload_identity_only`.

Binding v2 emits `verification_schema` version **2**, top-level
`producer_set_schema`/`scenario_set_schema`, and the two matched set digests in
each compact `verified_artifacts` row. Its fixed scope is
`variance_run_artifact_content_run_identity_and_producer_scenario_sets_only`.
It does not dump per-binding reports or preimages. V1 canonical manifest bytes,
hashes, result version/bytes, qualifications, and public fingerprint call behavior
remain unchanged.

New identity output and v2 results qualify their hashes as
`integrity_only_not_authentication_producer_execution_attestation_or_owner_authorization`.
Their `not_verified` list removes **only** `producer_set_sha256` and
`scenario_set_sha256` from the v1 list above. Budget scenario identity, runner
control/environment equality, statistical/variance analysis, independent
executions, other evidence, retention, approval/authentication, baseline acceptance
and performance enforcement remain unverified. `performance_enforcement.state`
is always `not_eligible`; the derivation command uses reason
`artifact_identity_derivation_only`, and v2 retains
`artifact_binding_verification_only`. Identity derivation/matching is not a
performance pass or producer execution attestation.

Both commands emit canonical UTF-8/LF JSON independent of stdout locale. Exit 0
means derivation/matching only; input failure is exit 1 with stderr only and no
partial JSON or handled-input traceback; usage errors are exit 2 and help is
exit 0. Module input failures raise `BaselineError`.

All existing guards remain: manifest input 1 MiB/depth 64/128 mappings, artifact
checksum inventory 4 MiB/10,000 entries, streamed hashing and local-only Git.
Existing report-parser resource characteristics remain unchanged. Documents and
artifacts must remain unchanged; no atomic snapshot or race-proof filesystem
guarantee is claimed. No command rewrites contracts, approvals, policies or
artifacts, collects measurements, fetches locators, invokes Cargo/network,
selects a latest run, or makes a clock-based decision. Broader PR-601 acceptance
and budget-identity verification remain unfinished with no ledger promotion.
See [ADR 0007](docs/adr/0007-producer-scenario-identities.md).

The measured report below remains the June 2026 baseline; the harness does not
replace those numbers until a clean, committed-SHA run is recorded.

## June 2026 report commands

The comparable local + Docker matrix was run with:

```bash
scripts/bench-suite.sh all \
  --duration 5 \
  --warmup 1 \
  --clients 1 \
  --depths 1,2,4,8,16 \
  --operations read,mixed \
  --output-dir bench-output/stress-20260603-full-suite
```

The same script can run either side independently:

```bash
scripts/bench-suite.sh local
scripts/bench-suite.sh docker
```

The local stress script now runs the stress binary in release mode by default:

```bash
cargo run --release -p rusty-modbus-benchmarks --bin stress-test -- ...
```

Codec/framing microbenchmarks are run with:

```bash
scripts/bench-local.sh codec --quick --noplot
scripts/bench-local.sh store --quick --noplot
scripts/bench-local.sh handler --quick --noplot
scripts/bench-local.sh tcp-pipelined --quick --noplot
scripts/bench-local.sh tcp-pool --quick --noplot
```

Criterion quick-mode rows are run through the individual script modes instead
of `scripts/bench-local.sh all --quick --noplot` because Cargo runs the library
bench harness first in package-wide mode, and that harness rejects Criterion's
`--quick` flag.

## Results

### Read Holding Registers

Workload: repeated FC 0x03 reads of 10 holding registers.

| Runtime | In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Local release | 1 | 62,096 | 310,480 | 0.015 | 0.021 | 0.031 | 0.047 | 0.109 | 0 | 0 |
| Local release | 2 | 120,735 | 603,675 | 0.015 | 0.024 | 0.031 | 0.052 | 0.096 | 0 | 1 |
| Local release | 4 | 159,272.6 | 796,363 | 0.021 | 0.047 | 0.069 | 0.099 | 0.978 | 0 | 1 |
| Local release | 8 | 200,458 | 1,002,290 | 0.035 | 0.073 | 0.103 | 0.158 | 3.329 | 0 | 1 |
| Local release | 16 | 230,327.6 | 1,151,638 | 0.064 | 0.113 | 0.147 | 0.204 | 3.519 | 0 | 1 |
| Alpine container | 1 | 62,492.2 | 312,461 | 0.008 | 0.055 | 0.089 | 0.126 | 0.216 | 0 | 0 |
| Alpine container | 2 | 74,327.2 | 371,636 | 0.017 | 0.072 | 0.107 | 0.147 | 0.277 | 0 | 0 |
| Alpine container | 4 | 90,633.6 | 453,168 | 0.037 | 0.102 | 0.137 | 0.190 | 0.775 | 0 | 0 |
| Alpine container | 8 | 125,461.8 | 627,309 | 0.058 | 0.127 | 0.165 | 0.217 | 0.514 | 0 | 0 |
| Alpine container | 16 | 275,310.2 | 1,376,551 | 0.054 | 0.089 | 0.122 | 0.177 | 0.640 | 0 | 0 |
| Distroless container | 1 | 94,829.6 | 474,148 | 0.008 | 0.024 | 0.026 | 0.032 | 11.967 | 0 | 0 |
| Distroless container | 2 | 158,864.6 | 794,323 | 0.011 | 0.022 | 0.029 | 0.039 | 0.142 | 0 | 0 |
| Distroless container | 4 | 178,032.4 | 890,162 | 0.023 | 0.031 | 0.039 | 0.050 | 0.144 | 0 | 0 |
| Distroless container | 8 | 247,732.4 | 1,238,662 | 0.031 | 0.046 | 0.054 | 0.065 | 0.294 | 0 | 0 |
| Distroless container | 16 | 286,203 | 1,431,015 | 0.053 | 0.083 | 0.094 | 0.108 | 0.241 | 0 | 0 |

### Mixed Read/Write

Workload: alternating FC 0x03 reads and FC 0x06 write-single-register requests.

| Runtime | In-flight | Throughput ops/s | Total ops | p50 ms | p95 ms | p99 ms | p99.9 ms | Max ms | Errors | RSS delta MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Local release | 1 | 59,054 | 295,270 | 0.016 | 0.021 | 0.031 | 0.039 | 0.093 | 0 | 0 |
| Local release | 2 | 112,511.2 | 562,556 | 0.016 | 0.026 | 0.035 | 0.057 | 0.177 | 0 | 1 |
| Local release | 4 | 161,249.6 | 806,248 | 0.022 | 0.040 | 0.064 | 0.091 | 5.027 | 0 | 1 |
| Local release | 8 | 250,629.2 | 1,253,146 | 0.031 | 0.044 | 0.051 | 0.066 | 0.141 | 0 | 1 |
| Local release | 16 | 251,765 | 1,258,825 | 0.058 | 0.117 | 0.159 | 0.204 | 0.309 | 0 | 1 |
| Alpine container | 1 | 98,961.8 | 494,809 | 0.008 | 0.024 | 0.026 | 0.053 | 0.780 | 0 | 0 |
| Alpine container | 2 | 142,523.8 | 712,619 | 0.011 | 0.023 | 0.033 | 0.074 | 2.179 | 0 | 0 |
| Alpine container | 4 | 175,937.6 | 879,688 | 0.022 | 0.034 | 0.048 | 0.090 | 5.763 | 0 | 0 |
| Alpine container | 8 | 247,175.8 | 1,235,879 | 0.031 | 0.047 | 0.059 | 0.080 | 0.167 | 0 | 0 |
| Alpine container | 16 | 287,541.8 | 1,437,709 | 0.052 | 0.082 | 0.093 | 0.108 | 0.267 | 0 | 0 |
| Distroless container | 1 | 95,229.4 | 476,147 | 0.008 | 0.025 | 0.026 | 0.032 | 11.223 | 0 | 0 |
| Distroless container | 2 | 155,972 | 779,860 | 0.011 | 0.023 | 0.029 | 0.038 | 0.140 | 0 | 0 |
| Distroless container | 4 | 178,149.2 | 890,746 | 0.023 | 0.031 | 0.038 | 0.049 | 0.134 | 0 | 0 |
| Distroless container | 8 | 250,089.6 | 1,250,448 | 0.031 | 0.046 | 0.054 | 0.069 | 1.391 | 0 | 0 |
| Distroless container | 16 | 287,946.4 | 1,439,732 | 0.052 | 0.082 | 0.093 | 0.105 | 0.378 | 0 | 0 |

### Docker Image Footprint

These image sizes were collected with `docker inspect` after local arm64 builds.

| Image | Target | Size |
|---|---|---:|
| `rusty-modbus:local` | `runtime` | 6.7 MB |
| `rusty-modbus:distroless` | `distroless` | 2.9 MB |
| `rusty-modbus-bench:alpine` | `benchmark` | 8.9 MB |
| `rusty-modbus-bench:distroless` | `benchmark-distroless` | 5.2 MB |

## Findings

- Single-connection pipelining still scales materially on local loopback. The
  local release run scaled from 62.1k to 230.3k ops/sec for reads and from
  59.1k to 251.8k ops/sec for mixed read/write.
- All 30 local/Docker rows completed with zero request errors.
- Tail latency rose as expected with deeper queues. p99 stayed below 0.17 ms
  for every local and Docker row in this matrix.
- RSS stayed effectively flat, with local measured deltas at 0-1 MiB across the
  matrix.
- The Docker runs are not an apples-to-apples replacement for native macOS
  numbers because Docker Desktop runs inside a Linux VM. In this environment
  distroless remained faster than native at most queue depths, Alpine lagged at
  depths 2-8 on the read-only workload, and both containers converged around
  286k-288k ops/sec at depth 16.
- The distroless runtime keeps the same functional smoke behavior as the Alpine
  runtime while cutting the local arm64 image footprint by roughly 56%.
- The distroless benchmark image is about 42% smaller than the Alpine benchmark
  image and was faster than Alpine on most rows in this local loopback suite.
- Docker Desktop produced isolated max-latency outliers at shallow distroless
  depth-1 rows, but the p99 and p99.9 values stayed low and no request errors
  were recorded.

## Codec and Zero-Copy Direction

The current codec already uses the most important zero-copy pattern for Modbus:
decode operates over caller-owned `&[u8]`, variable-length response payloads
borrow from that buffer, and owned response types slice `bytes::Bytes` instead of
copying payloads.

On the server side, the in-memory store now exposes direct wire-byte hooks for
FC 0x01/0x02 bit tables, FC 0x03/0x04 register tables, FC 0x14/0x15 file
records, and FC 0x18 FIFO queues. The handler allocates the final response PDU
once and lets the store write directly into the wire payload bytes, avoiding the
previous scratch buffers, queue clone, per-group `Vec<u16>` materialization, and
second response-encoding pass on common paths.

FC 0x2B / MEI 0x0E Read Device Identification now keeps the configured object
list on the stack, slices basic/regular selections without temporary vectors,
and pre-sizes the final response buffer.

FC 0x11 Report Server ID now lets direct-access stores append identification
bytes into the final response buffer, avoiding the previous cloned server-id
blob before response encoding.

FC 0x08 Diagnostics now lets stores append response data into the final
response buffer. The in-memory store uses this to echo Return Query Data from
borrowed request bytes instead of cloning the diagnostic payload first.

FC 0x0C Get Comm Event Log now lets stores append bounded event bytes into the
final response buffer while returning only the fixed status/counter metadata.
Existing stores that return an owned `CommEventLog` still work through the
default hook.

FC 0x14 Read File Record now builds the final response PDU directly while each
sub-response group is filled, avoiding the previous intermediate response-data
buffer and second encode/copy pass.

FC 0x15 Write File Record now validates sub-requests into a fixed stack buffer
bounded by the protocol's 0xFB-byte request-data cap. A one-register write group
is the smallest valid sub-request at 9 bytes, so the largest valid request can
contain 27 groups; this preserves the two-pass "validate before commit" behavior
without a heap `Vec` for group staging.

Packed coil/discrete helpers now work a byte at a time instead of repeatedly
dividing each bit index back into an output byte. This keeps the wire format
unchanged while reducing the dominant FC 0x01/0x02 read cost and the FC 0x0F
packed write unpack path.

The in-memory store now keeps coil and discrete-input tables in byte-backed bit
tables instead of `Vec<bool>`. That gives direct Modbus wire-byte paths a
single packed representation to read/write, while the public bool-slice methods
now pay a pack/unpack boundary cost.

`zerocopy` is already used where it is a strong fit: the fixed 7-byte MBAP
header is represented as a packed, network-endian wire-format type and the frame
decoder overlays it onto the read buffer before slicing the PDU. The benchmark
suite now includes MBAP decode with per-iteration allocation and MBAP decode
with a reused receive buffer so future changes can distinguish parser cost from
buffer allocation cost.

The PDU codec remains hand-written for now. Most PDU decode paths read a few
big-endian `u16` fields and then borrow the remaining payload. Extending
`zerocopy` into every small request/response body would add layout types and
derive requirements without an obvious copy to remove. `rkyv` is not a fit for
the Modbus wire path: it is designed for data serialized into rkyv's archived
layout, while Modbus is an external big-endian protocol format with per-function
validation rules.

The codec quick smoke was run with `scripts/bench-local.sh codec --quick --noplot`.
Treat these as hotspot-shape indicators, not release-grade Criterion baselines:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| Max FC 0x10 request decode | 1.58 ns | Decode validates the envelope and borrows payload bytes. |
| Max FC 0x03 response decode | 1.48 ns | Response decode borrows register payload bytes. |
| Max FC 0x03 response decode + register iteration | 44.2 ns | Register value access, not decode, is the first payload-sized cost. |
| Owned `Bytes` FC 0x03 dispatch | 12.3 ns | Owned slicing/refcount path is still small. |
| MBAP decode, fresh buffer per iteration | 29.8 ns | Includes receive-buffer allocation/copy shape. |
| MBAP decode, reused buffer | 13.9 ns | Isolates framing/parser work more closely. |
| Max register write unpack to `Vec<u16>` | 62.5 ns | Server write materialization is larger than decode. |
| Max coil write unpack to `Vec<bool>` | 371 ns | Packed-bit expansion is the strongest current allocation/copy candidate. |

The packed store read/write quick smoke was run with
`scripts/bench-local.sh store --quick --noplot` after adding direct wire-byte
paths to the in-memory store:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| Max register write from `&[u16]` | 6.53 ns | Slice baseline for existing store API. |
| Max register write from wire bytes | 7.43 ns | Direct packed path avoids the previous temporary `Vec<u16>`. |
| Max register wire bytes via `Vec<u16>` | 68.7 ns | Approximate old handler shape. |
| Max register read to BE wire bytes | 28.9 ns | Store writes directly into the response payload buffer. |
| Max register read via `u16` buffer then pack | 37.9 ns | Approximate old handler shape; extra scratch copy/encode pass costs ~31%. |
| Max coil write from `&[bool]` | 295 ns | Bool-slice writes now pack into the byte-backed table. |
| Max coil write from packed wire bytes | 234 ns | Direct wire-byte writes merge packed bytes into the table. |
| Max coil packed bytes via `Vec<bool>` | 1.18 us | Approximate old handler shape; unpacking to bools and repacking is now clearly slower than the direct path. |
| Max coil read to packed wire bytes | 124 ns | Store slices packed table bytes directly into Modbus wire order. |
| Max coil read via bool buffer then pack | 1.37 us | Bool-slice reads now unpack from the packed table before the benchmark repacks to wire bytes. |
| Max FIFO read to BE wire bytes | 15.1 ns | Store writes the queue snapshot directly into the response payload buffer. |
| Max FIFO read via cloned `Vec<u16>` then pack | 27.9 ns | Approximate old handler shape; queue clone and second pack pass roughly double this microbench. |
| Max file-record read to BE wire bytes | 32.3 ns | Store writes a 122-register file sub-record directly into the response payload buffer. |
| Max file-record read via `u16` buffer then pack | 40.8 ns | Approximate old FC14 handler shape; extra scratch copy/encode pass costs ~26%. |
| Max file-record write from `&[u16]` | 12.3 ns | Slice baseline for existing store API. |
| Max file-record write from wire bytes | 11.2 ns | Direct FC15 path keeps borrowed request bytes through validation and avoids per-group allocation. |
| Max file-record wire bytes via `Vec<u16>` | 70.9 ns | Approximate old FC15 handler shape; per-group vector materialization dominates. |

The RTU-over-TCP CRC scan quick smoke was run with
`scripts/bench-local.sh codec rtu_tcp --quick --noplot` after changing the
frame-boundary scan to update CRC state incrementally:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| RTU/TCP FC 0x03 read request decode | 34.9 ns | Short-frame happy path remains tiny. |
| RTU/TCP max-size valid frame decode | 461 ns | Full-frame scan stays sub-microsecond. |
| RTU/TCP full corrupt buffer decode | 410 ns | No-match path now scans once instead of rehashing every prefix. |
| Old-style prefix rescan, full corrupt buffer | 42.4 us | Benchmark-only comparator for the previous scan strategy. |

The server handler quick smoke was run with
`scripts/bench-local.sh handler --quick --noplot` after adding direct
`process_request` baselines. These rows include request decode, protocol
validation, in-memory store access, and response construction, but exclude TCP,
TLS, RTU framing, and client-side work:

| Path | Quick-mode timing | Signal |
|---|---:|---|
| FC01 max coil read | 123 ns | Byte-backed table lets the store emit packed response bytes directly. |
| FC02 max discrete-input read | 126 ns | Shares the same byte-backed packed response path as FC01. |
| FC03 max holding-register read | 29.2 ns | Direct BE register response path keeps full-size reads small. |
| FC0F max coil write | 255 ns | Packed request bytes merge directly into the byte-backed coil table. |
| FC10 max register write | 29.9 ns | Direct BE write path avoids the old request-payload `Vec<u16>`. |
| FC14 two-group file read | 48.4 ns | Direct final-buffer construction removes the previous response-data buffer and encode pass. |
| FC15 two-group file write | 57.3 ns | Stack-bounded validation staging removes the previous group `Vec` while preserving atomic framing validation. |
| FC17 max read/write registers | 42.7 ns | Read half now writes directly into the final response bytes. |
| FC18 FIFO two-value read | 27.0 ns | Direct FIFO response path is comparable to simple register handlers. |
| FC08 return query data | 21.9 ns | Direct diagnostic append path echoes borrowed request bytes into the response. |
| FC0C get comm event log | 17.9 ns | Direct event-log append path writes bounded event bytes into the response buffer. |
| FC11 report server ID | 22.2 ns | Direct server-id append path avoids cloning the store blob before response construction. |
| FC2B basic device identification | 31.1 ns | Stack-backed object selection removes the previous object/filter/selection vectors. |

The pipelined TCP Criterion quick smoke was run with
`scripts/bench-local.sh tcp-pipelined --quick --noplot`. This benchmark reports
read-holding-register throughput for repeated batches at each in-flight depth:

| In-flight | Quick-mode throughput |
|---:|---:|
| 1 | 48.1 Kelem/s |
| 2 | 85.0 Kelem/s |
| 4 | 136 Kelem/s |
| 8 | 199 Kelem/s |
| 16 | 250 Kelem/s |

The most likely next performance wins are adjacent to, not inside, raw PDU
parsing:

- Keep Criterion baselines around maximum-size request decode, response
  dispatch, owned `Bytes` dispatch, register iteration, packed write paths, and
  server handler dispatch before changing parser internals.
- Continue evaluating diagnostics and device-identification paths where
  temporary vectors or store cloning still dominate more than borrowed decode.
- Add multi-client stress matrices to separate protocol overhead from Tokio task
  scheduling and connection scaling.
- Add allocation profiling for server handlers and Python bindings so zero-copy
  decisions target measured heap churn instead of parser aesthetics.

## Caveats

- These numbers are local loopback measurements on one developer machine. They
  are useful as a regression baseline and hotspot guide, not as cross-machine
  marketing numbers.
- The server and client run on the same host, so kernel scheduling and loopback
  behavior dominate more than real network latency.
- The matrix uses an in-memory store. Device, gateway, TLS, serial, and slow-store
  workloads need separate baselines.
- The run uses 5-second measurement windows for timely iteration. Release-facing
  comparisons should use longer windows and Criterion baselines where practical.
- The stress benchmark spawns a loopback server; restricted sandboxes may need
  explicit permission to bind local sockets.

## Next Benchmarks

- TCP/TLS/RTU-over-TCP comparison for read, write, and mixed workloads.
- Multi-client plus per-client in-flight matrix to separate connection scaling
  from single-connection pipelining.
- Python binding throughput against a Python baseline.
- Allocation profiling for server write handlers that currently unpack request
  payloads into temporary vectors before calling the datastore.
- Machine-readable benchmark history so future PRs can compare against this
  baseline automatically.
