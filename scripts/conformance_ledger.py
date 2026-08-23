"""Validate and render the canonical conformance ledger."""

from __future__ import annotations

import html
import json
import posixpath
import re
from collections import Counter
from pathlib import Path, PurePosixPath
from typing import Any, TypeGuard

SCHEMA_VERSION = 1
EVIDENCE_LEVELS = (
    "not-implemented",
    "implemented",
    "internally-verified",
    "interoperable",
    "formally-certified",
)
EVIDENCE_RANK = {level: index for index, level in enumerate(EVIDENCE_LEVELS)}
DISPOSITIONS = {"supported", "unsupported", "compatibility-deviation"}
STRENGTHS = {"MUST", "SHOULD", "MAY", "project-profile"}
CLASSIFICATIONS = {"normative", "project-profile", "extension"}
PROFILE_IDS = (
    "tcp-client",
    "tcp-server",
    "physical-rtu-client",
    "physical-rtu-responder",
    "gateway",
    "modbus-security",
    "simulator",
    "rtu-over-tcp-extension",
)
REQUIREMENT_IDS = tuple(
    [f"APP-{number:03d}" for number in range(1, 23)]
    + [f"TCP-{number:03d}" for number in range(1, 15)]
    + [f"RTU-{number:03d}" for number in range(1, 13)]
    + [f"EXT-{number:03d}" for number in range(1, 5)]
    + [f"SEC-{number:03d}" for number in range(1, 11)]
    + [f"CONF-{number:03d}" for number in range(1, 9)]
)
PROJECT_POLICY_ID = "rusty-modbus-project-policy"
TEST_GLOB = "crates/rusty-modbus-conformance/tests/spec_*.rs"
FORMAL_WORDING = re.compile(r"\b(certified|certification|conformance[- ]tested)\b", re.I)
FINDING_IDS = tuple(f"F-{number:03d}" for number in range(1, 30))
FINDING_PRIORITIES = {"P0", "P1", "P2", "P3"}
FINDING_STATUSES = {"open", "mitigated", "closed"}
FOLLOW_UP_IDS = tuple(f"ISSUE-{number}" for number in range(90, 94))
FOLLOW_UP_STATUSES = {"open", "closed"}


class LedgerError(Exception):
    """Raised when the ledger cannot be loaded or does not validate."""


def load_ledger(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise LedgerError(f"cannot load {path}: {error}") from error
    if not isinstance(value, dict):
        raise LedgerError("ledger root must be a JSON object")
    return value


def discover_live_tests(root: Path) -> set[str]:
    return {
        path.relative_to(root).as_posix()
        for path in root.glob(TEST_GLOB)
        if path.is_file()
    }


def _nonblank(value: object) -> TypeGuard[str]:
    return isinstance(value, str) and bool(value.strip())


def _ids(records: Any, collection: str, errors: list[str]) -> list[str]:
    if not isinstance(records, list):
        errors.append(f"{collection} must be a list")
        return []
    result: list[str] = []
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            errors.append(f"{collection}[{index}] must be an object")
            continue
        record_id = record.get("id")
        if not _nonblank(record_id):
            errors.append(f"{collection}[{index}].id must be non-blank")
            continue
        result.append(record_id)
    duplicates = sorted(item for item, count in Counter(result).items() if count > 1)
    if duplicates:
        errors.append(f"duplicate {collection} IDs: {', '.join(duplicates)}")
    return result


def _relative_existing_file(root: Path, value: Any, label: str, errors: list[str]) -> None:
    if not _nonblank(value):
        errors.append(f"{label} must be a non-blank repository-relative path")
        return
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or str(path) != value:
        errors.append(f"{label} must be a normalized repository-relative path: {value!r}")
        return
    if not (root / path).is_file():
        errors.append(f"{label} does not exist: {value}")


def _validate_evidence_artifacts(
    artifacts: Any, label: str, *, formal: bool, errors: list[str]
) -> None:
    if not isinstance(artifacts, list) or not artifacts:
        errors.append(f"{label} requires at least one evidence artifact")
        return
    for index, artifact in enumerate(artifacts):
        item = f"{label}[{index}]"
        if not isinstance(artifact, dict):
            errors.append(f"{item} must be an object")
            continue
        for field in ("reference", "version", "result", "scope"):
            if not _nonblank(artifact.get(field)):
                errors.append(f"{item}.{field} must be non-blank")
        if formal and artifact.get("authorized") is not True:
            errors.append(f"{item}.authorized must be true for formal evidence")


def _surface_has_profile_link(root: Path, path: str, profile_id: str) -> bool:
    text = (root / path).read_text(encoding="utf-8")
    pattern = re.compile(
        rf"\[[^\]]+\]\([^\s)]*ledger\.md#profile-{re.escape(profile_id)}\)"
    )
    return bool(pattern.search(text))


def validate_ledger(data: dict[str, Any], root: Path) -> list[str]:
    errors: list[str] = []
    if data.get("schema_version") != SCHEMA_VERSION:
        errors.append(
            f"unsupported schema_version {data.get('schema_version')!r}; expected {SCHEMA_VERSION}"
        )

    baseline = data.get("baseline")
    if not isinstance(baseline, dict):
        errors.append("baseline must be an object")
    else:
        for field in ("repository", "base_sha", "base_ref", "as_of", "review_seed"):
            if not _nonblank(baseline.get(field)):
                errors.append(f"baseline.{field} must be non-blank")
        if baseline.get("requirement_count") != len(REQUIREMENT_IDS):
            errors.append(f"baseline.requirement_count must be {len(REQUIREMENT_IDS)}")

    notice = data.get("certification_notice")
    if not _nonblank(notice):
        errors.append("certification_notice must be non-blank")

    collections = {
        name: data.get(name)
        for name in (
            "documents",
            "profiles",
            "requirements",
            "tests",
            "claims",
            "public_surfaces",
            "findings",
            "follow_ups",
        )
    }
    collection_ids = {
        name: _ids(records, name, errors) for name, records in collections.items()
    }
    documents = {
        record["id"]: record
        for record in collections["documents"] or []
        if isinstance(record, dict) and _nonblank(record.get("id"))
    }
    profiles = {
        record["id"]: record
        for record in collections["profiles"] or []
        if isinstance(record, dict) and _nonblank(record.get("id"))
    }
    requirements = {
        record["id"]: record
        for record in collections["requirements"] or []
        if isinstance(record, dict) and _nonblank(record.get("id"))
    }
    tests = {
        record["id"]: record
        for record in collections["tests"] or []
        if isinstance(record, dict) and _nonblank(record.get("id"))
    }
    claims = {
        record["id"]: record
        for record in collections["claims"] or []
        if isinstance(record, dict) and _nonblank(record.get("id"))
    }
    surfaces = {
        record["id"]: record
        for record in collections["public_surfaces"] or []
        if isinstance(record, dict) and _nonblank(record.get("id"))
    }

    for document_id, document in documents.items():
        for field in ("title", "revision", "published", "locator"):
            if not _nonblank(document.get(field)):
                errors.append(f"document {document_id}.{field} must be non-blank")
        if document.get("kind") not in {"normative", "test-specification", "project-policy"}:
            errors.append(f"document {document_id}.kind is invalid")
        if not (_nonblank(document.get("url")) or _nonblank(document.get("path"))):
            errors.append(f"document {document_id} requires url or path")
        if _nonblank(document.get("path")):
            _relative_existing_file(root, document["path"], f"document {document_id}.path", errors)

    actual_profiles = set(collection_ids["profiles"])
    expected_profiles = set(PROFILE_IDS)
    missing_profiles = sorted(expected_profiles - actual_profiles)
    extra_profiles = sorted(actual_profiles - expected_profiles)
    if missing_profiles:
        errors.append(f"missing profile IDs: {', '.join(missing_profiles)}")
    if extra_profiles:
        errors.append(f"extra profile IDs: {', '.join(extra_profiles)}")
    for profile_id, profile in profiles.items():
        for field in ("name", "scope", "owner"):
            if not _nonblank(profile.get(field)):
                errors.append(f"profile {profile_id}.{field} must be non-blank")
        if profile.get("anchor") != f"profile-{profile_id}":
            errors.append(f"profile {profile_id}.anchor must be profile-{profile_id}")
        if profile.get("transport_class") not in {
            "modbus-tcp",
            "physical-rtu",
            "gateway",
            "security",
            "simulator",
            "extension",
        }:
            errors.append(f"profile {profile_id}.transport_class is invalid")
    extension_profile = profiles.get("rtu-over-tcp-extension")
    if extension_profile and extension_profile.get("transport_class") != "extension":
        errors.append("rtu-over-tcp-extension must have transport_class extension")

    actual_requirements = set(collection_ids["requirements"])
    expected_requirements = set(REQUIREMENT_IDS)
    missing_requirements = sorted(expected_requirements - actual_requirements)
    extra_requirements = sorted(actual_requirements - expected_requirements)
    if missing_requirements:
        errors.append(f"missing requirement IDs: {', '.join(missing_requirements)}")
    if extra_requirements:
        errors.append(f"extra requirement IDs: {', '.join(extra_requirements)}")

    assessment_index: dict[tuple[str, str], dict[str, Any]] = {}
    for requirement_id, requirement in requirements.items():
        for field in ("title", "owner"):
            if not _nonblank(requirement.get(field)):
                errors.append(f"requirement {requirement_id}.{field} must be non-blank")
        classification = requirement.get("classification")
        strength = requirement.get("strength")
        if classification not in CLASSIFICATIONS:
            errors.append(f"requirement {requirement_id}.classification is invalid or blank")
        if strength not in STRENGTHS:
            errors.append(f"requirement {requirement_id}.strength is invalid or blank")
        if classification in {"project-profile", "extension"} and strength != "project-profile":
            errors.append(
                f"requirement {requirement_id} project classifications require project-profile strength"
            )
        if classification == "normative" and strength == "project-profile":
            errors.append(f"requirement {requirement_id} normative classification has project strength")

        source = requirement.get("source")
        if not isinstance(source, dict):
            errors.append(f"requirement {requirement_id}.source must be an object")
        else:
            document_id = source.get("document")
            revision = source.get("revision")
            locator = source.get("locator")
            if document_id not in documents:
                errors.append(f"requirement {requirement_id} has broken document reference {document_id!r}")
            else:
                if not _nonblank(revision):
                    errors.append(f"requirement {requirement_id}.source.revision must be non-blank")
                elif revision != documents[document_id].get("revision"):
                    errors.append(f"requirement {requirement_id} source revision does not match document")
            if not _nonblank(locator):
                errors.append(f"requirement {requirement_id}.source.locator must be non-blank")
            if classification in {"project-profile", "extension"} and document_id != PROJECT_POLICY_ID:
                errors.append(f"requirement {requirement_id} must cite the revisioned project policy")

        implementation_refs = requirement.get("implementation_refs")
        if not isinstance(implementation_refs, list):
            errors.append(f"requirement {requirement_id}.implementation_refs must be a list")
            implementation_refs = []
        for index, reference in enumerate(implementation_refs):
            label = f"requirement {requirement_id}.implementation_refs[{index}]"
            if not isinstance(reference, dict):
                errors.append(f"{label} must be an object")
                continue
            _relative_existing_file(root, reference.get("path"), f"{label}.path", errors)
            if "symbol" in reference and not _nonblank(reference.get("symbol")):
                errors.append(f"{label}.symbol must be non-blank when present")

        test_ids = requirement.get("test_ids")
        if not isinstance(test_ids, list):
            errors.append(f"requirement {requirement_id}.test_ids must be a list")
            test_ids = []
        if len(test_ids) != len(set(test_ids)):
            errors.append(f"requirement {requirement_id} repeats a test reference")
        for test_id in test_ids:
            if test_id not in tests:
                errors.append(f"requirement {requirement_id} has broken test reference {test_id!r}")

        gap = requirement.get("evidence_gap")
        if not test_ids and not isinstance(gap, dict):
            errors.append(f"requirement {requirement_id} requires tests or an evidence gap")
        if gap is not None:
            if not isinstance(gap, dict):
                errors.append(f"requirement {requirement_id}.evidence_gap must be an object or null")
            else:
                for field in ("detail", "owner", "follow_up"):
                    if not _nonblank(gap.get(field)):
                        errors.append(
                            f"requirement {requirement_id}.evidence_gap.{field} must be non-blank"
                        )
                gap_profiles = gap.get("profiles")
                if gap_profiles is not None:
                    if not isinstance(gap_profiles, list) or not gap_profiles:
                        errors.append(
                            f"requirement {requirement_id}.evidence_gap.profiles must be a non-empty list"
                        )
                    else:
                        for profile_id in gap_profiles:
                            if profile_id not in profiles:
                                errors.append(
                                    f"requirement {requirement_id}.evidence_gap has broken profile {profile_id!r}"
                                )

        assessments = requirement.get("assessments")
        if not isinstance(assessments, list) or not assessments:
            errors.append(f"requirement {requirement_id}.assessments must be a non-empty list")
            assessments = []
        assessment_profiles: list[str] = []
        for index, assessment in enumerate(assessments):
            label = f"requirement {requirement_id}.assessments[{index}]"
            if not isinstance(assessment, dict):
                errors.append(f"{label} must be an object")
                continue
            profile_id = assessment.get("profile")
            disposition = assessment.get("disposition")
            evidence = assessment.get("evidence")
            if isinstance(profile_id, str):
                assessment_profiles.append(profile_id)
            if profile_id not in profiles:
                errors.append(f"{label} has broken profile reference {profile_id!r}")
            if disposition not in DISPOSITIONS:
                errors.append(f"{label}.disposition is invalid or blank")
            if evidence not in EVIDENCE_RANK:
                errors.append(f"{label}.evidence is invalid or blank")
            if disposition in {"unsupported", "compatibility-deviation"} and not _nonblank(
                assessment.get("reason")
            ):
                errors.append(f"{label}.reason is required for {disposition}")
            if disposition == "supported" and evidence == "not-implemented":
                errors.append(f"{label} cannot be supported with not-implemented evidence")
            if disposition == "unsupported" and evidence != "not-implemented":
                errors.append(f"{label} unsupported disposition requires not-implemented evidence")
            if evidence in {"implemented", "internally-verified", "interoperable", "formally-certified"} and not implementation_refs:
                errors.append(f"{label} claims implementation without an implementation path")
            if evidence in {"interoperable", "formally-certified"}:
                _validate_evidence_artifacts(
                    assessment.get("independent_evidence"),
                    f"{label}.independent_evidence",
                    formal=False,
                    errors=errors,
                )
            if evidence == "formally-certified":
                _validate_evidence_artifacts(
                    assessment.get("formal_evidence"),
                    f"{label}.formal_evidence",
                    formal=True,
                    errors=errors,
                )
            if _nonblank(profile_id):
                assessment_index[(requirement_id, profile_id)] = assessment
        repeated_profiles = sorted(
            item for item, count in Counter(assessment_profiles).items() if count > 1
        )
        if repeated_profiles:
            errors.append(
                f"requirement {requirement_id} repeats profile assessments: {', '.join(repeated_profiles)}"
            )

        if requirement_id.startswith("EXT-"):
            if classification != "extension" or strength != "project-profile":
                errors.append(f"{requirement_id} must be a project-profile extension")
            if set(assessment_profiles) != {"rtu-over-tcp-extension"}:
                errors.append(f"{requirement_id} may only assess rtu-over-tcp-extension")

    live_tests = discover_live_tests(root)
    ledger_test_paths: list[str] = []
    for test_id, test in tests.items():
        path = test.get("path")
        _relative_existing_file(root, path, f"test {test_id}.path", errors)
        if _nonblank(path):
            ledger_test_paths.append(path)
        mapped = test.get("requirement_ids")
        if not isinstance(mapped, list):
            errors.append(f"test {test_id}.requirement_ids must be a list")
            mapped = []
        if len(mapped) != len(set(mapped)):
            errors.append(f"test {test_id} repeats a requirement reference")
        category = test.get("category")
        if not mapped and category not in {"project-only", "supporting"}:
            errors.append(f"test {test_id} must map a requirement or use an explicit category")
        if category not in {None, "project-only", "supporting"}:
            errors.append(f"test {test_id}.category is invalid")
        for requirement_id in mapped:
            if requirement_id not in requirements:
                errors.append(f"test {test_id} has broken requirement reference {requirement_id!r}")
            elif test_id not in requirements[requirement_id].get("test_ids", []):
                errors.append(f"test {test_id} mapping to {requirement_id} is not reciprocal")
    duplicate_paths = sorted(item for item, count in Counter(ledger_test_paths).items() if count > 1)
    if duplicate_paths:
        errors.append(f"duplicate test paths: {', '.join(duplicate_paths)}")
    ledger_paths = set(ledger_test_paths)
    missing_tests = sorted(live_tests - ledger_paths)
    extra_tests = sorted(ledger_paths - live_tests)
    if missing_tests:
        errors.append(f"unclassified live spec tests: {', '.join(missing_tests)}")
    if extra_tests:
        errors.append(f"ledger test paths not in live spec inventory: {', '.join(extra_tests)}")
    if isinstance(baseline, dict) and baseline.get("test_count") != len(live_tests):
        errors.append(f"baseline.test_count must be {len(live_tests)}")

    for claim_id, claim in claims.items():
        profile_id = claim.get("profile")
        minimum = claim.get("minimum_evidence")
        claim_kind = claim.get("kind")
        if profile_id not in profiles:
            errors.append(f"claim {claim_id} has broken profile reference {profile_id!r}")
        if claim_kind not in {"capability", "limitation"}:
            errors.append(f"claim {claim_id}.kind is invalid or blank")
        if not _nonblank(claim.get("text")):
            errors.append(f"claim {claim_id}.text must be non-blank")
        if minimum not in EVIDENCE_RANK:
            errors.append(f"claim {claim_id}.minimum_evidence is invalid or blank")
        requirement_ids = claim.get("requirement_ids")
        if not isinstance(requirement_ids, list) or not requirement_ids:
            errors.append(f"claim {claim_id}.requirement_ids must be a non-empty list")
            requirement_ids = []
        if len(requirement_ids) != len(set(requirement_ids)):
            errors.append(f"claim {claim_id} repeats a requirement reference")
        for requirement_id in requirement_ids:
            if requirement_id not in requirements:
                errors.append(f"claim {claim_id} has broken requirement reference {requirement_id!r}")
                continue
            assessment = (
                assessment_index.get((requirement_id, profile_id))
                if isinstance(profile_id, str)
                else None
            )
            if assessment is None:
                errors.append(
                    f"claim {claim_id} references {requirement_id} without a {profile_id} assessment"
                )
                continue
            if assessment.get("disposition") == "unsupported":
                if claim_kind != "limitation":
                    errors.append(
                        f"claim {claim_id} relies on unsupported requirement {requirement_id}"
                    )
                elif minimum != "not-implemented":
                    errors.append(
                        f"limitation claim {claim_id} must use not-implemented evidence"
                    )
            evidence = assessment.get("evidence")
            if minimum in EVIDENCE_RANK and evidence in EVIDENCE_RANK:
                if EVIDENCE_RANK[minimum] > EVIDENCE_RANK[evidence]:
                    errors.append(
                        f"claim {claim_id} threshold {minimum} exceeds {requirement_id} evidence {evidence}"
                    )
        public_surface_ids = claim.get("public_surface_ids")
        if not isinstance(public_surface_ids, list) or not public_surface_ids:
            errors.append(f"claim {claim_id}.public_surface_ids must be a non-empty list")
            public_surface_ids = []
        for surface_id in public_surface_ids:
            if surface_id not in surfaces:
                errors.append(f"claim {claim_id} has broken public surface reference {surface_id!r}")
            elif profile_id not in surfaces[surface_id].get("profiles", []):
                errors.append(f"claim {claim_id} surface {surface_id} does not track {profile_id}")
        if FORMAL_WORDING.search(str(claim.get("text", ""))):
            if minimum != "formally-certified":
                errors.append(f"claim {claim_id} uses formal wording without formal threshold")
        if minimum == "formally-certified":
            _validate_evidence_artifacts(
                claim.get("formal_evidence"),
                f"claim {claim_id}.formal_evidence",
                formal=True,
                errors=errors,
            )
    claimed_profiles = {
        claim.get("profile") for claim in claims.values() if isinstance(claim.get("profile"), str)
    }
    missing_profile_claims = sorted(set(PROFILE_IDS) - claimed_profiles)
    if missing_profile_claims:
        errors.append(f"profiles without a claim: {', '.join(missing_profile_claims)}")

    for surface_id, surface in surfaces.items():
        path = surface.get("path")
        _relative_existing_file(root, path, f"public surface {surface_id}.path", errors)
        profile_ids = surface.get("profiles")
        if not isinstance(profile_ids, list) or not profile_ids:
            errors.append(f"public surface {surface_id}.profiles must be a non-empty list")
            profile_ids = []
        for profile_id in profile_ids:
            if profile_id not in profiles:
                errors.append(f"public surface {surface_id} has broken profile {profile_id!r}")
            elif _nonblank(path) and (root / path).is_file():
                if not _surface_has_profile_link(root, path, profile_id):
                    errors.append(
                        f"public surface {surface_id} does not link ledger.md#profile-{profile_id}"
                    )

    actual_findings = set(collection_ids["findings"])
    expected_findings = set(FINDING_IDS)
    missing_findings = sorted(expected_findings - actual_findings)
    extra_findings = sorted(actual_findings - expected_findings)
    if missing_findings:
        errors.append(f"missing finding IDs: {', '.join(missing_findings)}")
    if extra_findings:
        errors.append(f"extra finding IDs: {', '.join(extra_findings)}")
    for finding in collections["findings"] or []:
        if not isinstance(finding, dict) or not _nonblank(finding.get("id")):
            continue
        finding_id = finding["id"]
        if finding_id not in expected_findings:
            continue
        for field in ("title", "confidence", "owner"):
            if not _nonblank(finding.get(field)):
                errors.append(f"finding {finding_id}.{field} must be non-blank")
        priority = finding.get("priority")
        if not isinstance(priority, str) or priority not in FINDING_PRIORITIES:
            errors.append(f"finding {finding_id}.priority is invalid")
        status = finding.get("status")
        if not isinstance(status, str) or status not in FINDING_STATUSES:
            errors.append(f"finding {finding_id}.status is invalid")
        elif status != "open" and not _nonblank(finding.get("status_reason")):
            errors.append(f"finding {finding_id}.status_reason must be non-blank")
        closure_packages = finding.get("primary_closure_packages")
        if not isinstance(closure_packages, list) or not closure_packages:
            errors.append(
                f"finding {finding_id}.primary_closure_packages must be a non-empty list"
            )
            closure_packages = []
        elif not all(_nonblank(item) for item in closure_packages):
            errors.append(
                f"finding {finding_id}.primary_closure_packages must contain non-blank strings"
            )
            closure_packages = []
        else:
            if len(closure_packages) != len(set(closure_packages)):
                errors.append(f"finding {finding_id} repeats a primary closure package")
            if closure_packages != sorted(closure_packages):
                errors.append(
                    f"finding {finding_id}.primary_closure_packages must be sorted"
                )
        requirement_ids = finding.get("requirement_ids")
        if not isinstance(requirement_ids, list) or not requirement_ids:
            errors.append(f"finding {finding_id}.requirement_ids must be a non-empty list")
            requirement_ids = []
        if all(isinstance(item, str) for item in requirement_ids):
            if len(requirement_ids) != len(set(requirement_ids)):
                errors.append(f"finding {finding_id} repeats a requirement reference")
        else:
            errors.append(f"finding {finding_id}.requirement_ids must contain strings")
            requirement_ids = []
        for requirement_id in requirement_ids:
            if requirement_id not in requirements:
                errors.append(f"finding {finding_id} has broken requirement {requirement_id!r}")

    actual_follow_ups = set(collection_ids["follow_ups"])
    expected_follow_ups = set(FOLLOW_UP_IDS)
    missing_follow_ups = sorted(expected_follow_ups - actual_follow_ups)
    extra_follow_ups = sorted(actual_follow_ups - expected_follow_ups)
    if missing_follow_ups:
        errors.append(f"missing follow_up IDs: {', '.join(missing_follow_ups)}")
    if extra_follow_ups:
        errors.append(f"extra follow_up IDs: {', '.join(extra_follow_ups)}")
    for record in collections["follow_ups"] or []:
        if not isinstance(record, dict) or not _nonblank(record.get("id")):
            continue
        record_id = record["id"]
        if record_id not in expected_follow_ups:
            continue
        for field in ("title", "owner", "follow_up", "url"):
            if not _nonblank(record.get(field)):
                errors.append(f"follow_up {record_id}.{field} must be non-blank")
        status = record.get("status")
        if not isinstance(status, str) or status not in FOLLOW_UP_STATUSES:
            errors.append(f"follow_up {record_id}.status is invalid")
        elif status != "open" and not _nonblank(record.get("status_reason")):
            errors.append(f"follow_up {record_id}.status_reason must be non-blank")
        requirement_ids = record.get("requirement_ids")
        if not isinstance(requirement_ids, list) or not requirement_ids:
            errors.append(f"follow_up {record_id}.requirement_ids must be a non-empty list")
            requirement_ids = []
        if all(isinstance(item, str) for item in requirement_ids):
            if len(requirement_ids) != len(set(requirement_ids)):
                errors.append(f"follow_up {record_id} repeats a requirement reference")
        else:
            errors.append(f"follow_up {record_id}.requirement_ids must contain strings")
            requirement_ids = []
        for requirement_id in requirement_ids:
            if requirement_id not in requirements:
                errors.append(f"follow_up {record_id} has broken requirement {requirement_id!r}")
    return errors


def _md(value: Any) -> str:
    return html.escape(str(value), quote=False).replace("|", "&#124;").replace("\n", " ")


def _source_link(document: dict[str, Any]) -> str:
    target = document.get("url")
    if not target:
        target = posixpath.relpath(document["path"], "docs/conformance")
    return f"[{_md(document['id'])}]({_md(target)})"


def _reference_text(reference: dict[str, Any]) -> str:
    path = f"`{_md(reference['path'])}`"
    symbol = reference.get("symbol")
    return f"{path} (`{_md(symbol)}`)" if symbol else path


def render_markdown(data: dict[str, Any]) -> str:
    documents = {record["id"]: record for record in data["documents"]}
    requirements = {record["id"]: record for record in data["requirements"]}
    claims_by_profile: dict[str, list[dict[str, Any]]] = {item: [] for item in PROFILE_IDS}
    for claim in data["claims"]:
        claims_by_profile[claim["profile"]].append(claim)

    lines = [
        "<!-- GENERATED by scripts/check-conformance-ledger.py; DO NOT EDIT. -->",
        "# Conformance profiles and evidence ledger",
        "",
        "> Generated from [`ledger.json`](ledger.json). Do not edit this file by hand.",
        "",
        data["certification_notice"],
        "",
        "## Baseline",
        "",
        f"- Repository: `{_md(data['baseline']['repository'])}`",
        f"- Base: `{_md(data['baseline']['base_ref'])}` at `{_md(data['baseline']['base_sha'])}`",
        f"- Inventory date: `{_md(data['baseline']['as_of'])}`",
        f"- Review seed: {_md(data['baseline']['review_seed'])}",
        f"- Requirements: {len(data['requirements'])}",
        f"- Conformance test files: {len(data['tests'])}",
        "- Evidence in this seed records repository implementation and mappings; test-file existence does not prove execution.",
        "",
        "## Evidence scale",
        "",
        "| Level | Meaning |",
        "|---|---|",
        "| `not-implemented` | Required behavior or evidence is absent. |",
        "| `implemented` | A repository implementation reference exists. |",
        "| `internally-verified` | A recorded repository test execution verifies the requirement. |",
        "| `interoperable` | Independent implementation or tool evidence is recorded. |",
        "| `formally-certified` | Authorized evidence and its certification scope are recorded. |",
        "",
        "Disposition is separate: `supported`, `unsupported`, or `compatibility-deviation`.",
        "",
        "## Source documents",
        "",
        "| ID | Document | Revision | Published | Locator |",
        "|---|---|---|---|---|",
    ]
    for document in sorted(data["documents"], key=lambda item: item["id"]):
        lines.append(
            f"| {_source_link(document)} | {_md(document['title'])} | `{_md(document['revision'])}` | {_md(document['published'])} | {_md(document['locator'])} |"
        )

    lines.extend(["", "## Profiles", ""])
    for profile_id in PROFILE_IDS:
        profile = next(item for item in data["profiles"] if item["id"] == profile_id)
        profile_requirements = [
            requirement
            for requirement in data["requirements"]
            if any(item["profile"] == profile_id for item in requirement["assessments"])
        ]
        lines.extend(
            [
                f'<a id="profile-{profile_id}"></a>',
                f"### {_md(profile['name'])} (`{profile_id}`)",
                "",
                _md(profile["scope"]),
                "",
                "**Claims**",
                "",
                "| Claim | Kind | Minimum evidence | Requirement IDs |",
                "|---|---|---|---|",
            ]
        )
        for claim in sorted(claims_by_profile[profile_id], key=lambda item: item["id"]):
            requirement_text = ", ".join(f"`{item}`" for item in claim["requirement_ids"])
            lines.append(
                f"| `{_md(claim['id'])}`: {_md(claim['text'])} | `{_md(claim['kind'])}` | `{_md(claim['minimum_evidence'])}` | {requirement_text} |"
            )
        lines.extend(
            [
                "",
                "**Requirement assessments**",
                "",
                "| ID | Requirement | Disposition | Evidence | Gap or deviation |",
                "|---|---|---|---|---|",
            ]
        )
        for requirement in sorted(profile_requirements, key=lambda item: REQUIREMENT_IDS.index(item["id"])):
            assessment = next(
                item for item in requirement["assessments"] if item["profile"] == profile_id
            )
            detail = assessment.get("reason")
            if not detail and requirement.get("evidence_gap"):
                gap = requirement["evidence_gap"]
                gap_profiles = gap.get("profiles")
                if not gap_profiles or profile_id in gap_profiles:
                    detail = gap["detail"]
            lines.append(
                f"| [`{requirement['id']}`](#requirement-{requirement['id'].lower()}) | {_md(requirement['title'])} | `{assessment['disposition']}` | `{assessment['evidence']}` | {_md(detail or '—')} |"
            )
        lines.append("")

    lines.extend(["## Requirement trace", ""])
    current_family = None
    family_names = {
        "APP": "Application protocol and data model",
        "TCP": "Modbus/TCP and MBAP",
        "RTU": "Physical serial RTU",
        "EXT": "RTU-over-TCP extension",
        "SEC": "Modbus/TCP Security",
        "CONF": "Verification and release evidence",
    }
    for requirement_id in REQUIREMENT_IDS:
        requirement = requirements[requirement_id]
        family = requirement_id.split("-", 1)[0]
        if family != current_family:
            if current_family is not None:
                lines.append("")
            lines.extend(
                [
                    f"### {family_names[family]}",
                    "",
                    "| ID | Strength | Source | Implementation | Tests / gap |",
                    "|---|---|---|---|---|",
                ]
            )
            current_family = family
        source = requirement["source"]
        document = documents[source["document"]]
        source_text = (
            f"{_source_link(document)} `{_md(source['revision'])}`, {_md(source['locator'])}"
        )
        implementation = "; ".join(
            _reference_text(item) for item in requirement["implementation_refs"]
        ) or "—"
        evidence_parts = []
        if requirement["test_ids"]:
            evidence_parts.append(", ".join(f"`{item}`" for item in requirement["test_ids"]))
        if requirement["evidence_gap"]:
            gap = requirement["evidence_gap"]
            evidence_parts.append(f"Gap: {_md(gap['detail'])} ({_md(gap['follow_up'])})")
        evidence = "; ".join(evidence_parts)
        lines.extend(
            [
                f'<a id="requirement-{requirement_id.lower()}"></a>',
                f"| `{requirement_id}` — {_md(requirement['title'])} | `{requirement['strength']}` | {source_text} | {implementation} | {evidence} |",
            ]
        )

    lines.extend(
        [
            "",
            "## Conformance test inventory",
            "",
            "Mappings identify intended coverage. They do not assert that a test executed.",
            "",
            "| Test ID | Path | Requirement IDs / category |",
            "|---|---|---|",
        ]
    )
    for test in sorted(data["tests"], key=lambda item: item["path"]):
        mapping = ", ".join(f"`{item}`" for item in test["requirement_ids"])
        if not mapping:
            mapping = f"`{test['category']}`"
        lines.append(f"| `{test['id']}` | `{test['path']}` | {mapping} |")

    lines.extend(
        [
            "",
            "## Findings",
            "",
            "| ID | Priority | Confidence | Status | Owner | Primary closure | Requirements | Finding | Resolution |",
            "|---|---|---|---|---|---|---|---|---|",
        ]
    )
    for finding in sorted(data["findings"], key=lambda item: item["id"]):
        closure = ", ".join(f"`{_md(item)}`" for item in finding["primary_closure_packages"])
        requirement_ids = ", ".join(f"`{_md(item)}`" for item in finding["requirement_ids"])
        resolution = finding.get("status_reason", "—")
        lines.append(
            f"| `{_md(finding['id'])}` | `{_md(finding['priority'])}` | {_md(finding['confidence'])} | `{_md(finding['status'])}` | {_md(finding['owner'])} | {closure} | {requirement_ids} | {_md(finding['title'])} | {_md(resolution)} |"
        )

    lines.extend(
        [
            "",
            "## Linked issue follow-ups",
            "",
            "| ID | Status | Owner | Follow-up | Summary | Resolution |",
            "|---|---|---|---|---|---|",
        ]
    )
    for record in sorted(data["follow_ups"], key=lambda item: item["id"]):
        record_id = f"[{record['id']}]({record['url']})" if record.get("url") else record["id"]
        resolution = record.get("status_reason", "—")
        lines.append(
            f"| {record_id} | `{_md(record['status'])}` | {_md(record['owner'])} | {_md(record['follow_up'])} | {_md(record['title'])} | {_md(resolution)} |"
        )

    lines.extend(
        [
            "",
            "## Updating this view",
            "",
            "Edit `ledger.json`, then run:",
            "",
            "```console",
            "python3 scripts/check-conformance-ledger.py --write",
            "python3 scripts/check-conformance-ledger.py --check",
            "```",
            "",
            "See [`schema.md`](schema.md) for claim thresholds, extension classification, and the update policy.",
            "",
        ]
    )
    return "\n".join(lines)


def check_generated_view(data: dict[str, Any], current: str) -> list[str]:
    first = render_markdown(data)
    second = render_markdown(data)
    errors: list[str] = []
    if first != second:
        errors.append("ledger.md rendering is nondeterministic")
    if current != first:
        errors.append(
            "docs/conformance/ledger.md is stale; run "
            "python3 scripts/check-conformance-ledger.py --write"
        )
    return errors
