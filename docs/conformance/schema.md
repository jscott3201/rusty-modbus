# Conformance ledger schema and policy

`ledger.json` is the maintained source for requirement, profile, evidence, and
claim data. `ledger.md` is generated from it and must not be edited by hand.
This policy is schema version 1.

## Stable identifiers

Requirement IDs are repository-facing evidence identifiers. Once assigned, an
ID must not be reused or renumbered. A retired requirement stays in the ledger
with its final disposition and an explanation. Version 1 contains these closed
ranges:

- `APP-001` through `APP-022`
- `TCP-001` through `TCP-014`
- `RTU-001` through `RTU-012`
- `EXT-001` through `EXT-004`
- `SEC-001` through `SEC-010`
- `CONF-001` through `CONF-008`

Document, profile, test, claim, public-surface, finding, and follow-up IDs are
also unique within their collections.

## Version 1 records

The top-level object contains:

- `schema_version`: integer `1`.
- `baseline`: repository, revision, inventory counts, and review-seed provenance.
- `certification_notice`: the limit on claims based on repository evidence.
- `documents`: revisioned normative documents and the revisioned project policy.
- `profiles`: the eight fixed claim scopes.
- `requirements`: normative, project-profile, and extension requirements.
- `tests`: the live `spec_*.rs` inventory and its mappings.
- `claims`: profile-scoped public claims with minimum evidence thresholds.
- `public_surfaces`: files whose profile links are checked.
- `findings`: the stable F-001 through F-029 risk-register identities and status.
- `follow_ups`: linked repository issues that constrain claims.

The initial review seed was supplied outside the repository in a gitignored
planning bundle. It is historical grounding, not a path or input required by
the validator in a clean checkout. Canonical requirement and finding records
live in `ledger.json`; the validator enforces fixed ID inventories and structural
constraints without duplicating record content or current status.

Every requirement records a paraphrased title, classification, strength,
revisioned source and locator, owner, implementation references, test IDs,
optional evidence gap, and one or more profile assessments. Implementation and
test references are repository-relative paths with optional symbols. Line
numbers are not accepted as anchors because they become stale during unrelated
edits. An evidence gap may include `profiles` when the gap applies only to
specific assessments.

Normative strength is one of `MUST`, `SHOULD`, `MAY`, or `project-profile`.
`project-profile` is used only for repository policy or an explicitly labeled
extension. It must not be used to turn implementation preference into a
standard requirement.

## Project-profile and extension classification

Project-profile requirements govern repository evidence, release claims,
lifecycle bounds, and other decisions that are not direct protocol clauses.
Their revisioned source is this policy. RTU-over-TCP is a project extension: it
is not physical RTU and it is not Modbus/TCP. Every `EXT-*` row is classified as
`extension` and is assessed only by the `rtu-over-tcp-extension` profile.

The fixed profile IDs are:

- `tcp-client`
- `tcp-server`
- `physical-rtu-client`
- `physical-rtu-responder`
- `gateway`
- `modbus-security`
- `simulator`
- `rtu-over-tcp-extension`

An assessment exists only where a requirement is relevant to a profile. Its
disposition is separate from its evidence. Disposition is `supported`,
`unsupported`, or `compatibility-deviation`; unsupported and deviation entries
include a reason. This separation prevents implemented compatibility behavior
from being reported as implementation of the normative behavior.

## Evidence and claims

Evidence has this exact low-to-high order:

1. `not-implemented`
2. `implemented`
3. `internally-verified`
4. `interoperable`
5. `formally-certified`

`implemented` means a repository implementation reference exists. A test path
is an intended evidence mapping, not proof that the test ran. Promotion to
`internally-verified` requires a recorded execution artifact. `interoperable`
also requires independent implementation or tool evidence with a named version
and result. `formally-certified` requires authorized evidence and an explicit
certification scope.

Claims name one profile, a `capability` or `limitation` kind, a minimum evidence
level, and the requirements on which the claim depends. A claim cannot exceed
the lowest evidence among those profile assessments. Unsupported requirements
cannot support a capability claim. A limitation may identify unsupported work
only at `not-implemented`. No aggregate claim can infer interoperability or
certification from internal tests.

Repository tests and private use of a conformance tool do not authorize a claim
that the Modbus Organization tested or certified this project. Formal wording
is permitted only when the ledger contains authorized evidence and its exact
scope. The Modbus Organization describes this limit at
<https://www.modbus.org/conformance-testing>.

Tracked public surfaces are split into raw blocks at blank lines. Validation
collapses whitespace within each block but does not parse Markdown or infer
polarity. The fixed formal vocabulary is `certified`, `certification`,
`conformance-tested`, and `conformance tested`, matched case-insensitively. A
block containing that vocabulary is valid only in one of these forms:

- **Canonical notice:** no formal-claim marker, and the normalized block exactly
  matches `certification_notice` from `ledger.json`.
- **Canonical formal claim:** exactly one
  `<!-- rusty-modbus-formal-claim: claim-tcp-client -->` marker, with the
  canonical claim ID substituted as needed. Removing that exact
  marker and normalizing whitespace must produce the canonical claim's exact
  text. The claim must cover the surface and profile, be a capability with a
  `formally-certified` threshold, and satisfy the evidence rules above.

Case, punctuation, extra prose, and inline Markdown remain significant after
whitespace normalization. Arbitrary negative wording is not exempt. A valid
marker without formal wording is orphaned, and malformed or multiple markers
are rejected. The marker references the canonical claim; it does not duplicate
claim text or evidence outside `ledger.json`.

## Test inventory

The validator derives the test inventory from
`crates/rusty-modbus-conformance/tests/spec_*.rs`. Every live file must appear
once in `tests` and must map to at least one requirement or carry the explicit
`project-only` or `supporting` category. Every requirement must name test IDs or
an evidence gap with detail, owner, and follow-up package.

## Finding register

Finding IDs `F-001` through `F-029` are stable defect identities. Each record
contains its title, priority, confidence, status, owner, primary closure package
list, and relevant requirement IDs. An ID cannot be reused for a different
defect. Priority is `P0`, `P1`, `P2`, or `P3`. Primary closure packages are a
nonempty, unique, sorted list of nonblank strings. Requirement IDs are nonempty,
unique, and refer to ledger requirements.

Finding status is `open`, `mitigated`, or `closed`. A mitigated or closed record
includes a nonblank `status_reason`. Status transitions are made in
`ledger.json`; they do not require validator-source changes. As seed regression
decisions, version 1 records `F-023` as `closed` by this ledger and `F-027` as
`mitigated`, with final closure assigned to PR-704. All other findings are
currently `open`.

Follow-up IDs `ISSUE-90` through `ISSUE-93` are the fixed linked-issue inventory.
Each record has a nonblank title, owner, follow-up description, URL, and a
nonempty unique list of valid requirement IDs. Follow-up status is `open` or
`closed`; a closed record includes a nonblank `status_reason`. Follow-up content
and status are authoritative in `ledger.json`.

## Update workflow

1. Edit `ledger.json`; do not edit `ledger.md`.
2. Keep IDs stable and cite the exact document revision and section, table, or
   figure. Paraphrase the requirement instead of copying specification text.
3. Add implementation and test mappings. If proof is absent, retain the lower
   evidence level and record a gap.
4. Generate the human view with
   `python3 scripts/check-conformance-ledger.py --write`.
5. Run `python3 scripts/test-conformance-ledger.py` and
   `python3 scripts/check-conformance-ledger.py --check`.

Protocol, profile, public-claim, and conformance-test changes must update the
ledger in the same pull request. A later release audit may close a finding, but
creating the ledger does not itself prove interoperability or certification.
