#!/usr/bin/env python3
"""Focused tests for conformance ledger validation and generation."""

from __future__ import annotations

import copy
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

import conformance_ledger as ledger  # noqa: E402


class ConformanceLedgerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.canonical = ledger.load_ledger(ROOT / "docs/conformance/ledger.json")

    def assert_invalid(self, data: dict, message: str) -> None:
        errors = ledger.validate_ledger(data, ROOT)
        self.assertTrue(
            any(message in error for error in errors),
            f"expected {message!r} in:\n" + "\n".join(errors),
        )

    def scan_surface_text(
        self, text: str
    ) -> list[tuple[int, str, tuple[str, ...], int]]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "surface.md").write_text(text, encoding="utf-8")
            return ledger._surface_formal_blocks(root, "surface.md")

    def validate_surface_text(
        self, text: str, data: dict | None = None, path: str = "README.md"
    ) -> list[str]:
        blocks = self.scan_surface_text(text)

        def scan(
            _root: Path, candidate: str
        ) -> list[tuple[int, str, tuple[str, ...], int]]:
            return blocks if candidate == path else []

        with mock.patch.object(ledger, "_surface_formal_blocks", side_effect=scan):
            return ledger.validate_ledger(data or self.canonical, ROOT)

    def assert_surface_invalid(
        self, text: str, message: str, data: dict | None = None, path: str = "README.md"
    ) -> None:
        errors = self.validate_surface_text(text, data=data, path=path)
        self.assertTrue(
            any(message in error for error in errors),
            f"expected {message!r} in:\n" + "\n".join(errors),
        )

    def promoted_tcp_formal_claim(self) -> tuple[dict, dict]:
        data = copy.deepcopy(self.canonical)
        claim = next(item for item in data["claims"] if item["id"] == "claim-tcp-client")
        claim["text"] = "The TCP client is Modbus Organization certified."
        artifact = {
            "reference": "authorized-report",
            "version": "1",
            "result": "pass",
            "scope": "tcp-client",
            "authorized": True,
        }
        claim["minimum_evidence"] = "formally-certified"
        claim["formal_evidence"] = [copy.deepcopy(artifact)]
        requirements = {item["id"]: item for item in data["requirements"]}
        for requirement_id in claim["requirement_ids"]:
            assessment = next(
                item
                for item in requirements[requirement_id]["assessments"]
                if item["profile"] == "tcp-client"
            )
            assessment["evidence"] = "formally-certified"
            assessment["independent_evidence"] = [copy.deepcopy(artifact)]
            assessment["formal_evidence"] = [copy.deepcopy(artifact)]
        return data, claim

    def test_complete_seed_is_valid(self) -> None:
        self.assertEqual(ledger.validate_ledger(self.canonical, ROOT), [])

    def test_renderer_is_deterministic_and_current(self) -> None:
        first = ledger.render_markdown(self.canonical)
        second = ledger.render_markdown(self.canonical)
        self.assertEqual(first, second)
        current = (ROOT / "docs/conformance/ledger.md").read_text(encoding="utf-8")
        self.assertEqual(ledger.check_generated_view(self.canonical, current), [])

    def test_stale_generated_view_is_rejected(self) -> None:
        self.assertIn("stale", " ".join(ledger.check_generated_view(self.canonical, "stale\n")))

    def test_unsupported_schema_version_is_rejected(self) -> None:
        data = copy.deepcopy(self.canonical)
        data["schema_version"] = 2
        self.assert_invalid(data, "unsupported schema_version")

    def test_duplicate_collection_ids_are_rejected(self) -> None:
        for collection in (
            "documents",
            "profiles",
            "requirements",
            "tests",
            "claims",
            "findings",
            "follow_ups",
        ):
            with self.subTest(collection=collection):
                data = copy.deepcopy(self.canonical)
                data[collection].append(copy.deepcopy(data[collection][0]))
                self.assert_invalid(data, f"duplicate {collection} IDs")

    def test_exact_requirement_inventory_is_enforced(self) -> None:
        missing = copy.deepcopy(self.canonical)
        missing["requirements"].pop()
        self.assert_invalid(missing, "missing requirement IDs")
        extra = copy.deepcopy(self.canonical)
        extra["requirements"][0]["id"] = "APP-999"
        self.assert_invalid(extra, "extra requirement IDs")

    def test_exact_finding_inventory_is_enforced(self) -> None:
        missing = copy.deepcopy(self.canonical)
        missing["findings"].pop()
        self.assert_invalid(missing, "missing finding IDs")

        extra = copy.deepcopy(self.canonical)
        extra["findings"][0]["id"] = "F-999"
        self.assert_invalid(extra, "extra finding IDs")

    def test_finding_structure_is_enforced(self) -> None:
        mutations = (
            ("title", "", ".title must be non-blank"),
            ("priority", "P4", ".priority is invalid"),
            ("confidence", "", ".confidence must be non-blank"),
            ("owner", "", ".owner must be non-blank"),
            ("status", "pending", ".status is invalid"),
            (
                "primary_closure_packages",
                [],
                ".primary_closure_packages must be a non-empty list",
            ),
            (
                "primary_closure_packages",
                [""],
                ".primary_closure_packages must contain non-blank strings",
            ),
            (
                "primary_closure_packages",
                ["PR-100", "PR-100"],
                "repeats a primary closure package",
            ),
            (
                "primary_closure_packages",
                ["PR-200", "PR-100"],
                ".primary_closure_packages must be sorted",
            ),
        )
        for field, value, message in mutations:
            with self.subTest(field=field):
                data = copy.deepcopy(self.canonical)
                data["findings"][0][field] = value
                self.assert_invalid(data, message)

        missing_mapping = copy.deepcopy(self.canonical)
        missing_mapping["findings"][0]["requirement_ids"] = []
        self.assert_invalid(missing_mapping, ".requirement_ids must be a non-empty list")

        broken_mapping = copy.deepcopy(self.canonical)
        broken_mapping["findings"][0]["requirement_ids"].append("APP-999")
        self.assert_invalid(broken_mapping, "has broken requirement 'APP-999'")

        repeated_mapping = copy.deepcopy(self.canonical)
        repeated_mapping["findings"][0]["requirement_ids"].append(
            repeated_mapping["findings"][0]["requirement_ids"][0]
        )
        self.assert_invalid(repeated_mapping, "repeats a requirement reference")

        data = copy.deepcopy(self.canonical)
        finding = data["findings"][0]
        finding["status"] = "mitigated"
        self.assert_invalid(data, ".status_reason must be non-blank")

    def test_finding_status_transitions_are_ledger_driven(self) -> None:
        closed = copy.deepcopy(self.canonical)
        finding = closed["findings"][0]
        finding.update(
            title="Updated wording for the same stable finding",
            priority="P3",
            confidence="Reassessed from current source",
            owner="release maintainers",
            status="closed",
            status_reason="Closed by a later work package.",
            primary_closure_packages=["PR-999"],
        )
        self.assertEqual(ledger.validate_ledger(closed, ROOT), [])

        reopened = copy.deepcopy(self.canonical)
        finding = next(item for item in reopened["findings"] if item["id"] == "F-023")
        finding["status"] = "open"
        finding.pop("status_reason")
        self.assertEqual(ledger.validate_ledger(reopened, ROOT), [])

    def test_current_seed_finding_decisions_are_preserved(self) -> None:
        findings = {item["id"]: item for item in self.canonical["findings"]}
        self.assertEqual(findings["F-023"]["status"], "closed")
        self.assertEqual(findings["F-024"]["status"], "closed")
        self.assertEqual(findings["F-027"]["status"], "mitigated")
        self.assertEqual(findings["F-006"]["status"], "closed")
        self.assertEqual(findings["F-007"]["status"], "closed")
        self.assertEqual(findings["F-008"]["status"], "closed")
        self.assertEqual(findings["F-009"]["status"], "closed")

    def test_seeded_requirement_corrections_remain_distinct(self) -> None:
        requirements = {item["id"]: item for item in self.canonical["requirements"]}
        app_010 = requirements["APP-010"]
        self.assertIsNone(app_010["evidence_gap"])
        self.assertTrue(
            all(
                assessment["disposition"] == "supported"
                for assessment in app_010["assessments"]
                if assessment["profile"] != "physical-rtu-responder"
            )
        )

        app_011 = requirements["APP-011"]
        self.assertEqual(app_011["evidence_gap"]["follow_up"], "PR-402")
        self.assertEqual(app_011["evidence_gap"]["profiles"], ["gateway"])
        assessments = {item["profile"]: item for item in app_011["assessments"]}
        for profile_id in (
            "tcp-client",
            "physical-rtu-client",
            "rtu-over-tcp-extension",
        ):
            self.assertEqual(assessments[profile_id]["disposition"], "supported")
        self.assertEqual(assessments["gateway"]["disposition"], "compatibility-deviation")

        sec_004 = requirements["SEC-004"]
        self.assertEqual(sec_004["evidence_gap"]["follow_up"], "PR-501")
        self.assertEqual(sec_004["assessments"][0]["disposition"], "compatibility-deviation")
        self.assertIn(
            {"path": "crates/rusty-modbus-tls/src/config.rs"},
            sec_004["implementation_refs"],
        )
        security_claim = next(
            item for item in self.canonical["claims"] if item["id"] == "claim-modbus-security"
        )
        self.assertIn("by default", security_claim["text"])
        self.assertIn("compatibility opt-out", security_claim["text"])

    def test_gap_followups_match_review_work_packages(self) -> None:
        expected = {
            "APP-002": "PR-003",
            "APP-011": "PR-402",
            "APP-012": "PR-402",
            "APP-013": "PR-402",
            "APP-019": "PR-303",
            "TCP-006": "PR-201",
            "TCP-010": "PR-301, PR-302",
            "TCP-011": "PR-204, PR-301",
            "TCP-013": "PR-403",
            "RTU-001": "PR-101",
            "RTU-004": "PR-102, PR-103",
            "RTU-005": "PR-102",
            "RTU-007": "PR-101",
            "RTU-008": "PR-102",
            "RTU-009": "PR-103",
            "RTU-010": "PR-103",
            "RTU-012": "PR-702",
            "EXT-001": "PR-104, PR-704",
            "EXT-002": "PR-104",
            "EXT-003": "PR-104",
            "EXT-004": "PR-104",
            "SEC-004": "PR-501",
            "SEC-007": "PR-501, PR-502",
            "SEC-008": "PR-502",
            "SEC-009": "PR-501",
            "SEC-010": "PR-503",
            "CONF-001": "PR-001",
            "CONF-002": "PR-001, PR-703",
            "CONF-003": "PR-102, PR-104",
            "CONF-004": "PR-102, PR-103, PR-702",
            "CONF-005": "PR-703",
            "CONF-006": "PR-002, PR-601",
            "CONF-007": "PR-704",
            "CONF-008": "PR-704",
        }
        actual = {
            item["id"]: item["evidence_gap"]["follow_up"]
            for item in self.canonical["requirements"]
            if item["evidence_gap"]
        }
        self.assertEqual(actual, expected)

    def test_review_seed_and_conformance_spec_locator_are_checkout_safe(self) -> None:
        review_seed = self.canonical["baseline"]["review_seed"]
        self.assertIn("Externally supplied, gitignored", review_seed)
        self.assertIn("not a clean-checkout dependency", review_seed)
        self.assertNotIn("_plan/", review_seed)
        document = next(
            item
            for item in self.canonical["documents"]
            if item["id"] == "modbus-conformance-tests"
        )
        self.assertEqual(
            document["locator"],
            "Table of Contents; §§1–9, including §9 Protocol Test",
        )

    def test_source_revision_and_locator_are_required(self) -> None:
        data = copy.deepcopy(self.canonical)
        data["requirements"][0]["source"]["revision"] = ""
        data["requirements"][1]["source"]["locator"] = ""
        self.assert_invalid(data, "source.revision must be non-blank")
        self.assert_invalid(data, "source.locator must be non-blank")

    def test_blank_strength_evidence_disposition_and_owner_are_rejected(self) -> None:
        data = copy.deepcopy(self.canonical)
        requirement = data["requirements"][0]
        requirement["strength"] = ""
        requirement["owner"] = ""
        requirement["assessments"][0]["evidence"] = ""
        requirement["assessments"][1]["disposition"] = ""
        self.assert_invalid(data, ".strength is invalid or blank")
        self.assert_invalid(data, ".owner must be non-blank")
        self.assert_invalid(data, ".evidence is invalid or blank")
        self.assert_invalid(data, ".disposition is invalid or blank")

    def test_required_deviation_and_gap_details_are_rejected_when_blank(self) -> None:
        data = copy.deepcopy(self.canonical)
        requirement = next(
            item
            for item in data["requirements"]
            if any(a["disposition"] != "supported" for a in item["assessments"])
        )
        assessment = next(a for a in requirement["assessments"] if a["disposition"] != "supported")
        assessment["reason"] = ""
        gap_requirement = next(item for item in data["requirements"] if item["evidence_gap"])
        gap_requirement["evidence_gap"]["detail"] = ""
        self.assert_invalid(data, ".reason is required")
        self.assert_invalid(data, ".evidence_gap.detail must be non-blank")

    def test_claimed_paths_must_exist(self) -> None:
        data = copy.deepcopy(self.canonical)
        data["requirements"][0]["implementation_refs"][0]["path"] = "missing.rs"
        self.assert_invalid(data, "does not exist: missing.rs")

    def test_live_test_inventory_missing_and_extra_are_rejected(self) -> None:
        live = ledger.discover_live_tests(ROOT)
        with mock.patch.object(ledger, "discover_live_tests", return_value=live | {"extra/spec_new.rs"}):
            self.assert_invalid(self.canonical, "unclassified live spec tests")
        reduced = set(live)
        reduced.pop()
        with mock.patch.object(ledger, "discover_live_tests", return_value=reduced):
            self.assert_invalid(self.canonical, "ledger test paths not in live spec inventory")

    def test_unclassified_test_is_rejected(self) -> None:
        data = copy.deepcopy(self.canonical)
        data["tests"][0]["requirement_ids"] = []
        data["tests"][0]["category"] = None
        self.assert_invalid(data, "must map a requirement or use an explicit category")

    def test_broken_profile_requirement_test_and_claim_references_are_rejected(self) -> None:
        mutations = (
            (lambda data: data["requirements"][0]["assessments"][0].update(profile="missing"), "broken profile"),
            (
                lambda data: next(
                    item for item in data["requirements"] if item["id"] == "APP-011"
                )["evidence_gap"]["profiles"].append("missing"),
                "evidence_gap has broken profile",
            ),
            (lambda data: data["tests"][0]["requirement_ids"].append("APP-999"), "broken requirement"),
            (lambda data: data["requirements"][0]["test_ids"].append("TEST-missing"), "broken test reference"),
            (lambda data: data["claims"][0]["public_surface_ids"].append("surface-missing"), "broken public surface"),
        )
        for mutate, message in mutations:
            with self.subTest(message=message):
                data = copy.deepcopy(self.canonical)
                mutate(data)
                self.assert_invalid(data, message)

    def test_claim_threshold_cannot_exceed_requirement_evidence(self) -> None:
        data = copy.deepcopy(self.canonical)
        data["claims"][0]["minimum_evidence"] = "internally-verified"
        self.assert_invalid(data, "threshold internally-verified exceeds")

    def test_unsupported_requirement_cannot_support_capability_claim(self) -> None:
        data = copy.deepcopy(self.canonical)
        claim = next(item for item in data["claims"] if item["kind"] == "limitation")
        claim["kind"] = "capability"
        self.assert_invalid(data, "relies on unsupported requirement")

    def test_each_profile_requires_a_claim(self) -> None:
        data = copy.deepcopy(self.canonical)
        data["claims"].pop()
        self.assert_invalid(data, "profiles without a claim")

    def test_interoperability_requires_independent_evidence(self) -> None:
        data = copy.deepcopy(self.canonical)
        claim = data["claims"][0]
        requirement_id = claim["requirement_ids"][0]
        requirement = next(item for item in data["requirements"] if item["id"] == requirement_id)
        assessment = next(item for item in requirement["assessments"] if item["profile"] == claim["profile"])
        assessment["evidence"] = "interoperable"
        assessment["independent_evidence"] = []
        self.assert_invalid(data, "requires at least one evidence artifact")

    def test_formal_wording_requires_authorized_scoped_evidence(self) -> None:
        data = copy.deepcopy(self.canonical)
        data["claims"][0]["text"] = "This profile is certified."
        self.assert_invalid(data, "formal wording without formal threshold")

        data = copy.deepcopy(self.canonical)
        claim = data["claims"][0]
        claim["minimum_evidence"] = "formally-certified"
        claim["formal_evidence"] = [
            {"reference": "x", "version": "1", "result": "pass", "scope": "", "authorized": False}
        ]
        self.assert_invalid(data, ".scope must be non-blank")
        self.assert_invalid(data, ".authorized must be true")

    def test_canonical_notice_accepts_physical_line_wrapping(self) -> None:
        notice = self.canonical["certification_notice"]
        wrapped = notice.replace(" evidence ", " evidence\n").replace(
            " imply ", "\n  imply "
        )
        self.assertEqual(self.validate_surface_text(wrapped), [])

    def test_current_tracked_public_surfaces_satisfy_formal_block_grammar(self) -> None:
        root_blocks = ledger._surface_formal_blocks(ROOT, "README.md")
        self.assertEqual(len(root_blocks), 1)
        self.assertEqual(
            ledger._normalize_public_block(root_blocks[0][1]),
            ledger._normalize_public_block(self.canonical["certification_notice"]),
        )
        self.assertEqual(root_blocks[0][2:], ((), 0))
        self.assertEqual(ledger._surface_formal_blocks(ROOT, "docs/api.md"), [])
        self.assertEqual(
            ledger._surface_formal_blocks(ROOT, "crates/rusty-modbus-tls/README.md"),
            [],
        )
        self.assertEqual(ledger.validate_ledger(self.canonical, ROOT), [])

    def test_reviewed_contrastive_and_arbitrary_negative_wording_are_rejected(self) -> None:
        hostile_blocks = (
            "Although not every profile is certified, the TCP client is certified.",
            "The TCP client is not certified.",
            "No profile has formal certification.",
        )
        for text in hostile_blocks:
            with self.subTest(text=text):
                self.assert_surface_invalid(
                    text,
                    "unbound formal wording that does not exactly match certification_notice",
                )

    def test_unbound_formal_wording_and_notice_drift_are_rejected(self) -> None:
        notice = self.canonical["certification_notice"]
        hostile_blocks = (
            "The TCP client is Modbus Organization certified.",
            f"{notice} The TCP client is certified.",
            notice.replace("repository-scoped", "Repository-scoped"),
            f"**{notice}**",
        )
        for text in hostile_blocks:
            with self.subTest(text=text):
                self.assert_surface_invalid(
                    text,
                    "unbound formal wording that does not exactly match certification_notice",
                )

    def test_exact_formal_claim_binding_requires_formal_ledger_evidence(self) -> None:
        data = copy.deepcopy(self.canonical)
        claim = next(item for item in data["claims"] if item["id"] == "claim-tcp-client")
        claim["text"] = "The TCP client is Modbus Organization certified."
        block = f"{claim['text']} <!-- rusty-modbus-formal-claim: {claim['id']} -->"
        self.assert_surface_invalid(
            block,
            "without a formally-certified capability threshold",
            data=data,
        )

        promoted, claim = self.promoted_tcp_formal_claim()
        block = f"{claim['text']} <!-- rusty-modbus-formal-claim: {claim['id']} -->"
        self.assertEqual(self.validate_surface_text(block, data=promoted), [])

    def test_bound_formal_claim_text_must_match_canonical_text_exactly(self) -> None:
        data, claim = self.promoted_tcp_formal_claim()
        marker = f"<!-- rusty-modbus-formal-claim: {claim['id']} -->"
        hostile_text = (
            f"{claim['text']}!",
            claim["text"].replace("The TCP", "the TCP"),
            claim["text"].replace("certified", "**certified**"),
            f"{claim['text']} This sentence adds scope.",
        )
        for text in hostile_text:
            with self.subTest(text=text):
                self.assert_surface_invalid(
                    f"{text} {marker}",
                    f"formal wording does not exactly match claim {claim['id']}.text",
                    data=data,
                )

    def test_formal_claim_marker_failures_are_rejected(self) -> None:
        data, claim = self.promoted_tcp_formal_claim()
        text = claim["text"]
        marker = f"<!-- rusty-modbus-formal-claim: {claim['id']} -->"
        cases = (
            (f"{text} {marker} {marker}", "multiple formal-claim bindings", data, "README.md"),
            (
                f"{self.canonical['certification_notice']} {marker}",
                f"formal wording does not exactly match claim {claim['id']}.text",
                data,
                "README.md",
            ),
            (
                f"{text} <!-- rusty-modbus-formal-claim: claim-unknown -->",
                "binds formal wording to unknown claim claim-unknown",
                data,
                "README.md",
            ),
            (
                f"{text} {marker}",
                "which does not track this surface",
                data,
                "crates/rusty-modbus-tls/README.md",
            ),
            (f"Ordinary text. {marker}", "orphan formal-claim binding", data, "README.md"),
            (
                f"{text} <!-- rusty-modbus-formal-claim {claim['id']} -->",
                "malformed rusty-modbus-formal-claim marker",
                data,
                "README.md",
            ),
            (
                f"Ordinary text. <!-- rusty-modbus-formal claim: {claim['id']} -->",
                "malformed rusty-modbus-formal-claim marker",
                data,
                "README.md",
            ),
        )
        for block, message, case_data, path in cases:
            with self.subTest(message=message):
                self.assert_surface_invalid(block, message, data=case_data, path=path)

        wrong_profile = copy.deepcopy(data)
        wrong_profile_claim = next(
            item for item in wrong_profile["claims"] if item["id"] == claim["id"]
        )
        wrong_profile_claim["public_surface_ids"].append("tls-readme")
        self.assert_surface_invalid(
            f"{text} {marker}",
            "whose profile is not tracked by this surface",
            data=wrong_profile,
            path="crates/rusty-modbus-tls/README.md",
        )

    def test_extension_cannot_be_classified_as_normative_transport(self) -> None:
        data = copy.deepcopy(self.canonical)
        extension = next(item for item in data["requirements"] if item["id"] == "EXT-001")
        extension["classification"] = "normative"
        extension["strength"] = "MUST"
        extension["assessments"][0]["profile"] = "tcp-client"
        self.assert_invalid(data, "EXT-001 must be a project-profile extension")
        self.assert_invalid(data, "EXT-001 may only assess rtu-over-tcp-extension")

    def test_public_surface_requires_profile_scoped_link(self) -> None:
        with mock.patch.object(ledger, "_surface_has_profile_link", return_value=False):
            self.assert_invalid(self.canonical, "does not link ledger.md#profile-")

    def test_follow_up_inventory_and_structure_are_enforced(self) -> None:
        missing = copy.deepcopy(self.canonical)
        missing["follow_ups"].pop()
        self.assert_invalid(missing, "missing follow_up IDs")

        extra = copy.deepcopy(self.canonical)
        extra["follow_ups"][0]["id"] = "ISSUE-999"
        self.assert_invalid(extra, "extra follow_up IDs")

        for field in ("title", "owner", "follow_up", "url"):
            with self.subTest(field=field):
                data = copy.deepcopy(self.canonical)
                data["follow_ups"][0][field] = ""
                self.assert_invalid(data, f".{field} must be non-blank")

        invalid_status = copy.deepcopy(self.canonical)
        invalid_status["follow_ups"][0]["status"] = "resolved"
        self.assert_invalid(invalid_status, ".status is invalid")

        missing_mapping = copy.deepcopy(self.canonical)
        missing_mapping["follow_ups"][0]["requirement_ids"] = []
        self.assert_invalid(missing_mapping, ".requirement_ids must be a non-empty list")

        repeated_mapping = copy.deepcopy(self.canonical)
        repeated_mapping["follow_ups"][0]["requirement_ids"].append(
            repeated_mapping["follow_ups"][0]["requirement_ids"][0]
        )
        self.assert_invalid(repeated_mapping, "repeats a requirement reference")

        broken_mapping = copy.deepcopy(self.canonical)
        broken_mapping["follow_ups"][0]["requirement_ids"].append("APP-999")
        self.assert_invalid(broken_mapping, "has broken requirement 'APP-999'")

        closed = copy.deepcopy(self.canonical)
        closed["follow_ups"][0]["status"] = "closed"
        self.assert_invalid(closed, ".status_reason must be non-blank")
        closed["follow_ups"][0]["status_reason"] = "Closed by the linked issue."
        self.assertEqual(ledger.validate_ledger(closed, ROOT), [])
        self.assertIn("Closed by the linked issue.", ledger.render_markdown(closed))

if __name__ == "__main__":
    unittest.main(verbosity=2)
