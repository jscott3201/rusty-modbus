#!/usr/bin/env python3
"""Focused tests for the reproducible baseline harness."""

from __future__ import annotations

import copy
import contextlib
import hashlib
import io
import json
import math
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import unittest
import weakref
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

import baseline  # noqa: E402

SHA = "a" * 40


def stress_fixture(**overrides: object) -> dict:
    value = {
        "schema_version": 1,
        "transport": "tcp",
        "clients": 1,
        "in_flight": 8,
        "duration_secs": 1,
        "warmup_secs": 0,
        "operation": "read",
        "registers": 10,
        "total_ops": 100,
        "throughput_ops_sec": 100.0,
        "per_client_ops_sec": 100.0,
        "latency_ms": {
            "p50": 0.1,
            "p95": 0.2,
            "p99": 0.3,
            "p999": 0.4,
            "min": 0.05,
            "max": 0.5,
        },
        "errors": 0,
        "error_rate": 0.0,
        "retry_attempts": 0,
        "memory": {"rss_before_mb": 10, "rss_after_mb": 11, "delta_mb": 1},
    }
    value.update(overrides)
    return value


def expected_scenario(**overrides: object) -> dict:
    value = {
        "transport": "tcp",
        "operation": "read",
        "in_flight": 8,
        "clients": 1,
        "registers": 10,
        "duration_secs": 1,
        "warmup_secs": 0,
    }
    value.update(overrides)
    return value


def initialize_git_lock_fixture(
    root: Path,
    *,
    criterion_versions: tuple[str, ...] = ("0.5.1",),
    include_lock: bool = True,
    benchmark_targets: tuple[str, ...] = ("tcp_throughput",),
    include_benchmark_manifest: bool = True,
    benchmark_manifest_text: str | None = None,
) -> str:
    def git(*args: str) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            ("git", *args), cwd=root, check=True, capture_output=True
        )

    git("init", "--quiet")
    (root / ".gitignore").write_text("bench-output/\nrendered/\nreports/\n")
    (root / "fixture.txt").write_text("immutable target fixture\n")
    if include_lock:
        lines = ["version = 4", ""]
        if not criterion_versions:
            lines.extend(
                [
                    "[[package]]",
                    'name = "fixture"',
                    'version = "1.0.0"',
                    "",
                ]
            )
        for version in criterion_versions:
            lines.extend(
                [
                    "[[package]]",
                    'name = "criterion"',
                    f'version = "{version}"',
                    "",
                ]
            )
        (root / "Cargo.lock").write_text("\n".join(lines))
    if include_benchmark_manifest:
        benchmarks = root / "benchmarks"
        benchmarks.mkdir()
        if benchmark_manifest_text is None:
            manifest_lines = [
                "[package]",
                'name = "fixture-benchmarks"',
                'version = "0.0.0"',
                "",
            ]
            for target in benchmark_targets:
                manifest_lines.extend(
                    [
                        "[[bench]]",
                        f'name = "{target}"',
                        "harness = false",
                        "",
                    ]
                )
            benchmark_manifest_text = "\n".join(manifest_lines)
        (benchmarks / "Cargo.toml").write_text(benchmark_manifest_text)
    git("add", ".")
    git(
        "-c",
        "user.name=Benchmark Report Test",
        "-c",
        "user.email=benchmark-report@example.invalid",
        "commit",
        "--quiet",
        "-m",
        "immutable fixture",
    )
    return git("rev-parse", "HEAD").stdout.decode().strip()


def environment_fixture(runner_label: str = "unit-test") -> dict:
    return {
        "schema_version": 1,
        "collection_status": "complete",
        "runner": {"label": runner_label, "github": {"RUNNER_OS": "Linux"}},
        "platform": {
            "os": "Linux",
            "release": "fixture",
            "kernel": "fixture",
            "architecture": "x86_64",
        },
        "cpu": {"model": "fixture", "model_source": "fixture", "logical_count": 2},
        "power": {"availability": "unavailable", "value": None, "source": None},
        "tools": {
            "rustc": {"version": "rustc fixture", "host": "x86_64-unknown-linux-gnu"},
            "cargo": "cargo fixture",
            "python": {
                "version": "3.fixture",
                "implementation": "CPython",
                "executable": "/fixture/python3",
            },
        },
        "cargo_metadata": {
            "workspace_root": "/fixture",
            "target_directory": "/fixture/target",
            "workspace_member_count": 1,
            "packages": [],
        },
    }


def populate_benchmark_evidence(
    run: baseline.ArtifactRun,
    *,
    measurement_offset: float = 0.0,
    criterion_targets: tuple[str, ...] = ("tcp_throughput",),
) -> None:
    repetitions = 5 if run.mode == "bench-full" else 1
    scenarios = baseline.stress_scenarios(run.mode, repetitions)
    parsed_dir = run.run_dir / "stress" / "parsed"
    parsed_dir.mkdir(parents=True)
    for index, scenario in enumerate(scenarios, 1):
        sample = stress_fixture(
            operation=scenario["operation"],
            in_flight=scenario["in_flight"],
            warmup_secs=1,
            throughput_ops_sec=float(100 + index) + measurement_offset,
            per_client_ops_sec=float(100 + index) + measurement_offset,
        )
        sample["repetition"] = scenario["repetition"]
        command_id = f"{index:03d}-stress-{scenario['operation']}-d{scenario['in_flight']}-r1"
        sample["command_id"] = command_id
        run.stress_samples.append(sample)
        label = (
            f"stress-{scenario['operation']}-d{scenario['in_flight']}-"
            f"r{scenario['repetition']}"
        )
        baseline.write_json(parsed_dir / f"{label}.json", sample)

        command_dir = run.run_dir / "commands" / command_id
        command_dir.mkdir()
        stdout_path = command_dir / "command.stdout"
        stderr_path = command_dir / "command.stderr"
        raw_sample = dict(sample)
        raw_sample.pop("command_id")
        raw_sample.pop("repetition")
        stdout_path.write_text(json.dumps(raw_sample, sort_keys=True) + "\n")
        stderr_path.write_text("")
        command = {
            "schema_version": 1,
            "command_id": command_id,
            "label": label,
            "argv": ["fixture-stress", "--json"],
            "cwd": str(run.repo_root),
            "started_utc": "2026-01-01T00:00:00.000000Z",
            "ended_utc": "2026-01-01T00:00:01.000000Z",
            "duration_seconds": 1.0,
            "exit_code": 0,
            "status": "passed",
            "error": None,
            "env_overrides": {},
            "stdout_path": stdout_path.relative_to(run.repo_root).as_posix(),
            "stderr_path": stderr_path.relative_to(run.repo_root).as_posix(),
        }
        baseline.write_json(command_dir / "command.json", command)
        run.command_records.append(command)

    run.stress_aggregates = baseline.aggregate_stress_samples(run.stress_samples, scenarios)
    run.criterion_results = []
    for index, target in enumerate(sorted(criterion_targets), 1):
        criterion_home = run.run_dir / "criterion" / "raw" / f"{index:02d}-{target}"
        benchmark_id = "tcp_pipelined" if target == "tcp_throughput" else f"{target}/fixture"
        estimate = criterion_home / benchmark_id / "new" / "estimates.json"
        estimate.parent.mkdir(parents=True)
        baseline.write_json(
            estimate,
            {
                "mean": {
                    "confidence_interval": {
                        "confidence_level": 0.95,
                        "lower_bound": 9.0 + measurement_offset,
                        "upper_bound": 11.0 + measurement_offset,
                    },
                    "point_estimate": 10.0 + measurement_offset,
                    "standard_error": 0.1,
                }
            },
        )
        run.criterion_results.extend(
            baseline.parse_criterion_estimates(criterion_home, run.repo_root)
        )
    run.criterion_results.sort(key=lambda item: item["source"].encode("utf-8"))
    baseline.write_json(
        run.run_dir / "criterion" / "parsed-estimates.json", run.criterion_results
    )
    run.environment = environment_fixture(run.runner_label)


def shifted_report_fixture(
    report: dict,
    *,
    run_id: str = "candidate-report",
    runner_label: str = "candidate-runner",
    throughput_delta: float = 25.0,
    p99_delta: float = -0.05,
    criterion_delta: float = 4.0,
) -> dict:
    candidate = copy.deepcopy(report)
    candidate["run"]["run_id"] = run_id
    source_parts = candidate["source_artifact"]["path"].split("/")
    source_parts[-1] = run_id
    candidate["source_artifact"]["path"] = "/".join(source_parts)
    candidate["runner"]["label"] = runner_label
    candidate["runner"]["environment"]["runner_label"] = runner_label
    for scenario in candidate["scenarios"]:
        if scenario["kind"] == "tcp_stress":
            for metric_name, delta in (
                ("throughput", throughput_delta),
                ("p99_latency", p99_delta),
            ):
                statistics = scenario["metrics"][metric_name]["recorded_statistics"]
                for field in ("min", "median", "mean", "max"):
                    statistics[field] += delta
        else:
            estimate = scenario["metrics"]["mean_estimate"]
            for field in ("lower", "point", "upper"):
                estimate[field] += criterion_delta
    return candidate


def disabled_policy_fixture() -> dict:
    return {
        "activation_blockers": list(baseline.POLICY_ACTIVATION_BLOCKERS),
        "approval": None,
        "approved_baseline": None,
        "budget_rules": [],
        "control_profile": None,
        "policy_id": "rusty-modbus-benchmark-budget-policy",
        "policy_schema": {
            "name": baseline.POLICY_SCHEMA_NAME,
            "version": baseline.POLICY_SCHEMA_VERSION,
        },
        "policy_state": "disabled",
        "statistical_method": None,
        "thresholds": [],
        "variance_evidence": [],
    }


def controlled_evidence_contract_fixture() -> dict:
    runner_id = "synthetic-controlled-runner"
    profile = {"profile_id": "synthetic-control-profile", "version": "synthetic-v1"}
    method = {"method_id": "synthetic-effect-uncertainty", "version": "synthetic-v1"}
    approval_id = "synthetic-contract-approval"
    promotion_rule_id = "synthetic-explicit-promotion-rule"

    def retained(evidence_id: str, kind: str, digest_digit: int) -> dict:
        return {
            "evidence_id": evidence_id,
            "kind": kind,
            "locator": f"opaque:synthetic/{evidence_id}",
            "recorded_utc": "2026-01-01T00:00:00Z",
            "retained_until_utc": "2027-01-01T00:00:00Z",
            "sha256": f"{digest_digit:064x}",
        }

    def run(
        run_id: str,
        target_sha: str,
        artifact_evidence_id: str,
        producer_digest: str,
        scenario_digest: str,
    ) -> dict:
        return {
            "artifact_evidence_id": artifact_evidence_id,
            "control_profile": dict(profile),
            "producer_set_sha256": producer_digest,
            "run_id": run_id,
            "runner_id": runner_id,
            "sample_unit": baseline.CONTROLLED_SAMPLE_UNIT,
            "scenario_set_sha256": scenario_digest,
            "statistical_method": dict(method),
            "target_sha": target_sha,
        }

    target_old = "a" * 40
    target_current = "b" * 40
    producer_old = "1" * 64
    scenarios_old = "2" * 64
    producer_current = "3" * 64
    scenarios_current = "4" * 64
    document = {
        "approval": {
            "approval_id": approval_id,
            "approved_utc": "2026-02-01T00:00:00Z",
            "authority_id": "synthetic-owner-authority",
            "evidence_id": "evidence-approval-record",
            "scope_sha256": "0" * 64,
        },
        "approval_authority": {
            "authority_id": "synthetic-owner-authority",
            "evidence_id": "evidence-approval-authority",
            "validation_semantics": baseline.CONTROLLED_APPROVAL_VALIDATION_SEMANTICS,
            "version": "synthetic-v1",
        },
        "baseline_lifecycle": {
            "initial_state": "candidate",
            "promotion_rule_evidence_id": "evidence-promotion-rule",
            "promotion_rule_id": promotion_rule_id,
            "promotion_path": [
                {"from_state": from_state, "to_state": to_state}
                for from_state, to_state in baseline.CONTROLLED_BASELINE_PROMOTION_PATH
            ],
            "selection": "explicit_baseline_id_only",
            "supersession_transition": {
                "from_state": "approved",
                "to_state": "superseded",
            },
        },
        "baselines": [
            {
                "artifact_evidence_id": "evidence-run-old-a",
                "baseline_id": "synthetic-baseline-old",
                "promotion_chain": [
                    {
                        "evidence_id": "evidence-variance-old",
                        "from_state": "candidate",
                        "to_state": "variance_collected",
                        "variance_study_id": "synthetic-study-old",
                    },
                    {
                        "evidence_id": "evidence-promotion-old",
                        "from_state": "variance_collected",
                        "rule_evidence_id": "evidence-promotion-rule",
                        "rule_id": promotion_rule_id,
                        "to_state": "promotion_pending",
                    },
                    {
                        "approval_id": approval_id,
                        "evidence_id": "evidence-approval-record",
                        "from_state": "promotion_pending",
                        "to_state": "approved",
                    },
                ],
                "run_id": "synthetic-old-a",
                "state": "superseded",
                "supersession": {
                    "evidence_id": "evidence-supersession-old",
                    "from_state": "approved",
                    "successor_baseline_id": "synthetic-baseline-current",
                    "to_state": "superseded",
                },
                "target_sha": target_old,
            },
            {
                "artifact_evidence_id": "evidence-run-current-a",
                "baseline_id": "synthetic-baseline-current",
                "promotion_chain": [
                    {
                        "evidence_id": "evidence-variance-current",
                        "from_state": "candidate",
                        "to_state": "variance_collected",
                        "variance_study_id": "synthetic-study-current",
                    },
                    {
                        "evidence_id": "evidence-promotion-current",
                        "from_state": "variance_collected",
                        "rule_evidence_id": "evidence-promotion-rule",
                        "rule_id": promotion_rule_id,
                        "to_state": "promotion_pending",
                    },
                    {
                        "approval_id": approval_id,
                        "evidence_id": "evidence-approval-record",
                        "from_state": "promotion_pending",
                        "to_state": "approved",
                    },
                ],
                "run_id": "synthetic-current-a",
                "state": "approved",
                "supersession": None,
                "target_sha": target_current,
            },
        ],
        "budget_rules": [
            {
                "budget_rule_id": "synthetic-throughput-minimum",
                "direction": "minimum",
                "evidence_id": "evidence-budget-throughput",
                "limit": 1.0,
                "metric": "throughput",
                "scenario_identity": {
                    "identity_sha256": "5" * 64,
                    "match": "exact_complete_identity",
                    "scenario_id": "synthetic-tcp-read-depth-8",
                },
                "unit": "operations_per_second",
            },
            {
                "budget_rule_id": "synthetic-p99-maximum",
                "direction": "maximum",
                "evidence_id": "evidence-budget-p99",
                "limit": 1.0,
                "metric": "p99_latency",
                "scenario_identity": {
                    "identity_sha256": "6" * 64,
                    "match": "exact_complete_identity",
                    "scenario_id": "synthetic-tcp-mixed-depth-8",
                },
                "unit": "ms",
            },
        ],
        "contract_id": "synthetic-controlled-evidence-contract",
        "contract_schema": {
            "name": baseline.CONTROLLED_EVIDENCE_CONTRACT_SCHEMA_NAME,
            "version": baseline.CONTROLLED_EVIDENCE_CONTRACT_SCHEMA_VERSION,
        },
        "control": {
            "binding": {
                "evidence_ids": ["evidence-runner-profile-binding"],
                "profile_id": profile["profile_id"],
                "profile_version": profile["version"],
                "proof_semantics": baseline.CONTROLLED_BINDING_PROOF_SEMANTICS,
                "runner_id": runner_id,
            },
            "profile": {
                "evidence_ids": ["evidence-control-profile"],
                **profile,
            },
            "runner": {
                "evidence_ids": [
                    "evidence-controlled-runner-b",
                    "evidence-controlled-runner-a",
                ],
                "runner_id": runner_id,
            },
        },
        "evidence_retention": [
            retained("evidence-analysis-plan", "analysis_plan", 1),
            retained("evidence-approval-authority", "approval_authority", 2),
            retained("evidence-approval-record", "approval_record", 3),
            retained("evidence-promotion-current", "baseline_promotion", 4),
            retained("evidence-promotion-old", "baseline_promotion", 5),
            retained("evidence-promotion-rule", "baseline_promotion_rule", 6),
            retained("evidence-supersession-old", "baseline_supersession", 7),
            retained("evidence-run-current-a", "benchmark_artifact", 8),
            retained("evidence-run-current-b", "benchmark_artifact", 9),
            retained("evidence-run-old-a", "benchmark_artifact", 10),
            retained("evidence-run-old-b", "benchmark_artifact", 11),
            retained("evidence-budget-p99", "budget_rule", 12),
            retained("evidence-budget-throughput", "budget_rule", 13),
            retained("evidence-control-profile", "control_profile", 14),
            retained("evidence-controlled-runner-a", "controlled_runner", 15),
            retained("evidence-controlled-runner-b", "controlled_runner", 16),
            retained("evidence-runner-profile-binding", "runner_profile_binding", 17),
            retained("evidence-variance-current", "variance_analysis", 18),
            retained("evidence-variance-old", "variance_analysis", 19),
        ],
        "semantics": dict(baseline.CONTROLLED_EVIDENCE_CONTRACT_SEMANTICS),
        "statistical_method": {
            "analysis_plan_evidence_id": "evidence-analysis-plan",
            **method,
            "minimum_independent_runs": 2,
            "outputs": {"effect": "required", "uncertainty": "required"},
            "sample_unit": baseline.CONTROLLED_SAMPLE_UNIT,
            "scenario_alignment": "exact_complete_set",
        },
        "variance_studies": [
            {
                "analysis_evidence_id": "evidence-variance-old",
                "control_profile": dict(profile),
                "producer_set_sha256": producer_old,
                "runner_id": runner_id,
                "runs": [
                    run(
                        "synthetic-old-a",
                        target_old,
                        "evidence-run-old-a",
                        producer_old,
                        scenarios_old,
                    ),
                    run(
                        "synthetic-old-b",
                        target_old,
                        "evidence-run-old-b",
                        producer_old,
                        scenarios_old,
                    ),
                ],
                "scenario_set_sha256": scenarios_old,
                "statistical_method": dict(method),
                "study_id": "synthetic-study-old",
                "target_sha": target_old,
            },
            {
                "analysis_evidence_id": "evidence-variance-current",
                "control_profile": dict(profile),
                "producer_set_sha256": producer_current,
                "runner_id": runner_id,
                "runs": [
                    run(
                        "synthetic-current-a",
                        target_current,
                        "evidence-run-current-a",
                        producer_current,
                        scenarios_current,
                    ),
                    run(
                        "synthetic-current-b",
                        target_current,
                        "evidence-run-current-b",
                        producer_current,
                        scenarios_current,
                    ),
                ],
                "scenario_set_sha256": scenarios_current,
                "statistical_method": dict(method),
                "study_id": "synthetic-study-current",
                "target_sha": target_current,
            },
        ],
    }
    document["approval"]["scope_sha256"] = (
        baseline._controlled_evidence_contract_approval_scope_sha256(document)
    )
    return document


class ControlledEvidenceLoaderTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        # Canonicalize macOS /var aliases before computing repository-relative paths.
        self.root = Path(temporary.name).resolve() / "repository"
        scripts = self.root / "scripts"
        scripts.mkdir(parents=True)
        self.script = scripts / "baseline.py"
        shutil.copyfile(SCRIPTS / "baseline.py", self.script)
        self.relative = "inputs/contrôle evidence.json"
        self.path = self.root / self.relative
        self.path.parent.mkdir()
        self.document = controlled_evidence_contract_fixture()
        self.path.write_text(json.dumps(self.document), encoding="utf-8")
        policy_relative = "benchmarks/policy/benchmark-budget-policy-v1.json"
        self.policy = self.root / policy_relative
        self.policy.parent.mkdir(parents=True)
        shutil.copyfile(ROOT / policy_relative, self.policy)

    def inventory(self) -> dict:
        return {
            path.relative_to(self.root).as_posix(): (
                ("symlink", str(path.readlink())) if path.is_symlink()
                else ("file", path.read_bytes()) if path.is_file()
                else ("directory", None) if path.is_dir()
                else ("special", None)
            )
            for path in self.root.rglob("*")
        }

    def cli(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.script), *args],
            cwd=self.root.parent,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=10,
        )

    def test_loader_normalizes_permutations_without_writes(self) -> None:
        permuted = copy.deepcopy(self.document)
        for field in ("variance_studies", "baselines", "budget_rules", "evidence_retention"):
            permuted[field].reverse()
        for study in permuted["variance_studies"]:
            study["runs"].reverse()
        for part in ("runner", "profile", "binding"):
            permuted["control"][part]["evidence_ids"].reverse()
        self.path.write_text(json.dumps(permuted), encoding="utf-8")
        before = self.inventory()

        loaded = baseline.load_controlled_evidence_contract_file(self.root, self.relative)

        self.assertEqual(
            loaded, json.loads(baseline.controlled_evidence_contract_json_text(self.document))
        )
        for field, identity in (
            ("variance_studies", "study_id"),
            ("baselines", "baseline_id"),
            ("budget_rules", "budget_rule_id"),
            ("evidence_retention", "evidence_id"),
        ):
            identities = [item[identity] for item in loaded[field]]
            self.assertEqual(identities, sorted(identities))
        self.assertEqual(self.inventory(), before)

    def test_cli_success_from_alternate_cwd_is_structural_only_and_read_only(self) -> None:
        # A cwd-relative decoy must not be read; locators are deliberately nonexistent.
        decoy = self.root.parent / self.relative
        decoy.parent.mkdir()
        decoy.write_bytes(b"not the contract")
        before = self.inventory()

        result = self.cli("validate-controlled-evidence", self.relative)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout, "controlled evidence contract structurally valid (validation only)\n"
        )
        self.assertEqual(result.stderr, "")
        self.assertEqual(self.inventory(), before)
        self.assertEqual(decoy.read_bytes(), b"not the contract")

    def assert_input_failure(self, relative: str) -> None:
        before = self.inventory()
        with self.assertRaises(baseline.BaselineError):
            baseline.load_controlled_evidence_contract_file(self.root, relative)
        result = self.cli("validate-controlled-evidence", relative)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertTrue(result.stderr.startswith("baseline: "), result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertNotIn("structurally valid", result.stderr)
        self.assertNotIn("sensitive-document-value", result.stderr)
        self.assertEqual(self.inventory(), before)

    def test_explicit_local_regular_file_paths_are_required(self) -> None:
        for relative in (
            str(self.path), "C:/contract.json", "//server/contract.json", "",
            ".", "..", "./inputs/contrôle evidence.json", "inputs/../contract.json",
            "../contract.json", "inputs//contrôle evidence.json", "inputs/",
            "inputs\\contract.json", "missing.json", "inputs", "inputs/missing.json",
        ):
            with self.subTest(path=relative):
                self.assert_input_failure(relative)
        with self.assertRaises(baseline.BaselineError):
            baseline.load_controlled_evidence_contract_file(self.root, "bad\x00path")

    def test_symlink_files_and_ancestors_are_rejected(self) -> None:
        external = self.root.parent / "external.json"
        external.write_bytes(self.path.read_bytes())
        links = {
            "file-link.json": self.path,
            "external-link.json": external,
            "dangling.json": self.root / "missing.json",
            "ancestor-link": self.path.parent,
            "external-ancestor": self.root.parent,
        }
        try:
            for relative, target in links.items():
                (self.root / relative).symlink_to(target, target_is_directory=target.is_dir())
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        external_before = external.read_bytes()
        for relative in (
            "file-link.json", "external-link.json", "dangling.json",
            "ancestor-link/contrôle evidence.json", "external-ancestor/external.json",
        ):
            with self.subTest(path=relative):
                self.assert_input_failure(relative)
        self.assertEqual(external.read_bytes(), external_before)

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFO creation unavailable")
    def test_special_file_is_rejected_without_opening(self) -> None:
        os.mkfifo(self.root / "fifo.json")
        with mock.patch.object(Path, "open", side_effect=AssertionError("must not open FIFO")):
            with self.assertRaises(baseline.BaselineError):
                baseline.load_controlled_evidence_contract_file(self.root, "fifo.json")
        self.assert_input_failure("fifo.json")

    def test_malformed_json_encoding_and_numbers_fail_without_tracebacks(self) -> None:
        valid = self.path.read_bytes()
        cases = {
            "invalid-utf8": b'{"value":"\xff"}',
            "empty": b"",
            "truncated": b'{"sensitive-document-value":',
            "trailing": valid + b" false",
            "array-root": b"[]",
            "null-root": b"null",
            "string-root": b'"sensitive-document-value"',
            "duplicate-root": valid.replace(
                b'"contract_id":', b'"contract_id":"sensitive-document-value","contract_id":'
            ),
            "duplicate-nested": valid.replace(b'"version":', b'"version":1,"version":', 1),
            "deep-object": b'{"x":' * 1500 + b"0" + b"}" * 1500,
            "deep-array": b"[" * 1500 + b"0" + b"]" * 1500,
            "escaped-surrogate": valid.replace(b"opaque:synthetic/", b"opaque:\\ud800/", 1),
        }
        for token in (
            b"NaN", b"Infinity", b"-Infinity", b"1e9999", b"-1e9999",
            b"9" * 400, b"9" * 5000,
        ):
            cases[f"number-{token[:12]!r}"] = valid.replace(b'"limit": 1.0', b'"limit": ' + token, 1)
        for label, raw in cases.items():
            with self.subTest(case=label):
                self.path.write_bytes(raw)
                self.assert_input_failure(self.relative)

    def test_nesting_is_bounded_before_json_parsing(self) -> None:
        for opener, closer in ((b'{"x":', b"}"), (b"[", b"]")):
            with self.subTest(opener=opener):
                self.path.write_bytes(opener * 65 + b"0" + closer * 65)
                with mock.patch.object(
                    baseline.json, "loads", side_effect=AssertionError("must not parse")
                ), self.assertRaisesRegex(baseline.BaselineError, "nesting"):
                    baseline.load_controlled_evidence_contract_file(self.root, self.relative)
                self.assert_input_failure(self.relative)
        # The 64-container boundary reaches schema validation, not the depth guard.
        self.path.write_bytes(b'{"x":' * 64 + b"0" + b"}" * 64)
        with mock.patch.object(
            baseline, "validate_controlled_evidence_contract", return_value=["invalid schema"]
        ) as validate, self.assertRaises(baseline.BaselineError):
            baseline.load_controlled_evidence_contract_file(self.root, self.relative)
        validate.assert_called_once()

    def test_unrepresentable_numbers_are_rejected_before_schema_validation(self) -> None:
        for token in (b"1e9999", b"-1e9999", b"9" * 400, b"9" * 5000):
            with self.subTest(token=token[:12]):
                self.path.write_bytes(b'{"value":' + token + b"}")
                with mock.patch.object(
                    baseline, "validate_controlled_evidence_contract",
                    side_effect=AssertionError("must not validate unrepresentable numbers"),
                ), self.assertRaises(baseline.BaselineError):
                    baseline.load_controlled_evidence_contract_file(self.root, self.relative)

    def test_json_string_escapes_unicode_and_representable_numbers_are_preserved(self) -> None:
        document = copy.deepcopy(self.document)
        document["evidence_retention"][0]["locator"] = (
            'opaque:synthetic/é🧪"\\' + "[" * 80 + "]" * 80
        )
        # Large finite integers must remain integers, not rounded through float parsing.
        document["budget_rules"][0]["limit"] = 2 ** 53 + 1
        document["approval"]["scope_sha256"] = (
            baseline._controlled_evidence_contract_approval_scope_sha256(document)
        )
        self.path.write_text(json.dumps(document, ensure_ascii=False), encoding="utf-8")
        before = self.inventory()
        loaded = baseline.load_controlled_evidence_contract_file(self.root, self.relative)
        self.assertEqual(
            loaded, json.loads(baseline.controlled_evidence_contract_json_text(document))
        )
        result = self.cli("validate-controlled-evidence", self.relative)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertEqual(self.inventory(), before)

    def test_byte_limit_is_inclusive_and_checked_before_parsing(self) -> None:
        self.assertEqual(baseline.CONTROLLED_EVIDENCE_CONTRACT_MAX_BYTES, 1024 * 1024)
        raw = self.path.read_bytes()
        padded = raw + b" " * (1024 * 1024 - len(raw))
        self.path.write_bytes(padded)
        before = self.inventory()
        self.assertEqual(
            baseline.load_controlled_evidence_contract_file(self.root, self.relative),
            json.loads(baseline.controlled_evidence_contract_json_text(self.document)),
        )
        self.assertEqual(self.cli("validate-controlled-evidence", self.relative).returncode, 0)
        self.assertEqual(self.inventory(), before)
        self.path.write_bytes(padded + b" ")
        with mock.patch.object(baseline.json, "loads", side_effect=AssertionError("must not parse")):
            with self.assertRaisesRegex(baseline.BaselineError, "1 MiB"):
                baseline.load_controlled_evidence_contract_file(self.root, self.relative)
        self.assert_input_failure(self.relative)

    def test_read_is_bounded_and_io_errors_are_baseline_errors(self) -> None:
        source = mock.MagicMock()
        source.__enter__.return_value.read.return_value = self.path.read_bytes()
        with mock.patch.object(Path, "open", return_value=source) as opened:
            baseline.load_controlled_evidence_contract_file(self.root, self.relative)
        opened.assert_called_once_with("rb")
        source.__enter__.return_value.read.assert_called_once_with(1024 * 1024 + 1)
        for error in (PermissionError("denied"), FileNotFoundError("removed")):
            with self.subTest(error=type(error).__name__), mock.patch.object(
                Path, "open", side_effect=error
            ), self.assertRaises(baseline.BaselineError):
                baseline.load_controlled_evidence_contract_file(self.root, self.relative)

    def test_existing_schema_lifecycle_and_approval_validation_is_delegated(self) -> None:
        cases = [
            (("contract_schema", "version"), 2),
            (("contract_schema", "version"), True),
            (("sensitive-document-value",), "sensitive-document-value"),
            (("baseline_lifecycle", "selection"), "latest"),
            (("baselines", 1, "promotion_chain"), []),
            (("approval",), None),
            (("approval", "scope_sha256"), "f" * 64),
            (("approval", "authority_id"), "sensitive-document-value"),
            (("variance_studies", 0, "runs", 1, "artifact_evidence_id"), "evidence-run-old-a"),
            (("budget_rules", 0, "limit"), -1),
            (("evidence_retention", 0, "locator"), "sensitive-document-value with spaces"),
            (("baselines", 1, "run_id"), "sensitive-document-value\n"),
            (("approval_authority",), []),
        ]
        for path, invalid in cases:
            with self.subTest(path=path):
                document = copy.deepcopy(self.document)
                target = document
                for part in path[:-1]:
                    target = target[part]
                target[path[-1]] = invalid
                self.path.write_text(json.dumps(document), encoding="utf-8")
                with mock.patch.object(
                    baseline, "validate_controlled_evidence_contract",
                    wraps=baseline.validate_controlled_evidence_contract,
                ) as validate:
                    self.assert_input_failure(self.relative)
                    validate.assert_called_once_with(document)

    def test_no_command_network_locator_or_write_edges_are_used(self) -> None:
        before = self.inventory()
        actual_open = Path.open

        def input_only_open(path: Path, mode: str = "r"):
            self.assertEqual(path, self.path)
            self.assertEqual(mode, "rb")
            return actual_open(path, mode)

        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.ExitStack() as stack:
            for name in (
                "bootstrap_repository", "collect_environment", "run_mode", "run_benchmarks",
                "build_benchmark_report", "controlled_evaluate_artifacts", "write_json",
                "load_policy_file",
            ):
                stack.enter_context(mock.patch.object(
                    baseline, name, side_effect=AssertionError(f"forbidden edge: {name}")
                ))
            for name in ("subprocess.Popen", "socket.create_connection", "urllib.request.urlopen"):
                stack.enter_context(mock.patch(name, side_effect=AssertionError(name)))
            stack.enter_context(mock.patch.object(Path, "open", input_only_open))
            stack.enter_context(mock.patch.object(Path, "glob", side_effect=AssertionError("discovery")))
            stack.enter_context(mock.patch.object(Path, "rglob", side_effect=AssertionError("discovery")))
            stack.enter_context(mock.patch.object(baseline, "__file__", str(self.script)))
            stack.enter_context(contextlib.redirect_stdout(stdout))
            stack.enter_context(contextlib.redirect_stderr(stderr))
            baseline.load_controlled_evidence_contract_file(self.root, self.relative)
            self.assertEqual(baseline.main(["validate-controlled-evidence", self.relative]), 0)
        self.assertIn("structurally valid", stdout.getvalue())
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(self.inventory(), before)

    def test_help_and_usage_exit_contract(self) -> None:
        before = self.inventory()
        for args in (("--help",), ("validate-controlled-evidence", "--help")):
            with self.subTest(args=args):
                result = self.cli(*args)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("validate-controlled-evidence", result.stdout)
                self.assertEqual(result.stderr, "")
        for args in (
            ("validate-controlled-evidence",),
            ("validate-controlled-evidence", self.relative, "--latest"),
            ("validate-controlled-evidence", self.relative, "extra.json"),
        ):
            with self.subTest(args=args):
                result = self.cli(*args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertIn("usage:", result.stderr)
                self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(self.inventory(), before)


class BaselineHarnessTests(unittest.TestCase):
    def make_run(
        self,
        root: Path,
        *,
        run_id: str = "test-run",
        dirty: bool = False,
        allow_dirty: bool = False,
        target_sha: str = SHA,
        mode: str = "bench-smoke",
    ) -> baseline.ArtifactRun:
        run = baseline.ArtifactRun(
            repo_root=root,
            output_root=root / "bench-output",
            target_sha=target_sha,
            run_id=run_id,
            mode=mode,
            runner_label="unit-test",
            dirty=dirty,
            allow_dirty=allow_dirty,
        )
        run.create()
        return run

    def make_report_run(
        self,
        root: Path,
        *,
        run_id: str,
        criterion_versions: tuple[str, ...] = ("0.5.1",),
        include_lock: bool = True,
        target_sha: str | None = None,
        mode: str = "bench-smoke",
        benchmark_targets: tuple[str, ...] = ("tcp_throughput",),
    ) -> baseline.ArtifactRun:
        fixture_sha = initialize_git_lock_fixture(
            root,
            criterion_versions=criterion_versions,
            include_lock=include_lock,
            benchmark_targets=benchmark_targets,
        )
        return self.make_run(
            root,
            run_id=run_id,
            target_sha=target_sha or fixture_sha,
            mode=mode,
        )

    def assert_controlled_contract_invalid(self, document: dict) -> None:
        self.assertTrue(baseline.validate_controlled_evidence_contract(document))
        with self.assertRaises(baseline.BaselineError):
            baseline.controlled_evidence_contract_json_text(document)

    def test_full_sha_and_clean_tree_are_required(self) -> None:
        self.assertEqual(baseline.validate_full_sha(SHA), SHA)
        for invalid in ("abc", "A" * 40, "a" * 39, "g" * 40):
            with self.subTest(invalid=invalid), self.assertRaises(baseline.BaselineError):
                baseline.validate_full_sha(invalid)
        with self.assertRaises(baseline.BaselineError):
            baseline.enforce_clean_worktree(True, False)
        baseline.enforce_clean_worktree(False, False)
        baseline.enforce_clean_worktree(True, True)

    def test_bootstrap_rejects_untracked_source_but_ignores_ignored_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def git(*args: str) -> None:
                subprocess.run(
                    ("git", *args),
                    cwd=root,
                    check=True,
                    capture_output=True,
                )

            git("init", "--quiet")
            (root / ".gitignore").write_text("bench-output/\n.DS_Store\n")
            (root / "tracked.txt").write_text("tracked\n")
            git("add", ".gitignore", "tracked.txt")
            git(
                "-c",
                "user.name=Baseline Test",
                "-c",
                "user.email=baseline@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            )

            (root / "untracked-source.py").write_text("print('dirty')\n")
            repository, commands = baseline.bootstrap_repository(root)
            self.assertTrue(repository["dirty"])
            self.assertEqual(
                commands[1].argv,
                ("git", "status", "--porcelain", "--untracked-files=all"),
            )

            (root / "untracked-source.py").unlink()
            (root / "bench-output").mkdir()
            (root / "bench-output" / "result.json").write_text("{}\n")
            (root / ".DS_Store").write_bytes(b"ignored")
            repository, _ = baseline.bootstrap_repository(root)
            self.assertFalse(repository["dirty"])

    def test_dirty_override_is_recorded_as_invalid(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_run(root, dirty=True, allow_dirty=True)
            run.finalize()
            summary = json.loads((run.run_dir / "summary.json").read_text())
            self.assertFalse(summary["baseline_valid"])
            self.assertEqual(summary["status"], "invalid")
            self.assertIn("dirty non-ignored worktree", summary["invalid_reasons"])
            errors = baseline.validate_artifact(root, run.run_dir)
            self.assertIn("provenance.json records a dirty worktree", errors)
            self.assertIn("provenance.json baseline_eligible must be true", errors)
            self.assertIn("summary.json baseline_valid must be true", errors)

    def test_run_id_output_path_and_collision_validation(self) -> None:
        for invalid in ("../x", "/tmp/x", ".", "..", "space value", "x" * 65):
            with self.subTest(invalid=invalid), self.assertRaises(baseline.BaselineError):
                baseline.validate_run_id(invalid)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(
                baseline.resolve_output_root(root, "bench-output"),
                (root / "bench-output").resolve(),
            )
            with self.assertRaises(baseline.BaselineError):
                baseline.resolve_output_root(root, "../outside")
            run = self.make_run(root)
            collision = baseline.ArtifactRun(
                repo_root=root,
                output_root=root / "bench-output",
                target_sha=SHA,
                run_id="test-run",
                mode="bench-smoke",
                runner_label="unit-test",
                dirty=False,
                allow_dirty=False,
            )
            with self.assertRaises(baseline.BaselineError):
                collision.create()
            run.finalize()

    def test_json_csv_and_checksum_outputs_are_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.json"
            second = root / "second.json"
            baseline.write_json(first, {"z": 1, "a": [2, 3]})
            baseline.write_json(second, {"a": [2, 3], "z": 1})
            self.assertEqual(first.read_bytes(), second.read_bytes())

            aggregate = {
                "transport": "tcp",
                "operation": "read",
                "in_flight": 1,
                "clients": 1,
                "registers": 10,
                "repetitions": 1,
                "throughput_ops_sec": baseline.sample_statistics([10.0]),
                "p99_ms": baseline.sample_statistics([0.1]),
                "total_errors": 0,
                "retry_attempts": 0,
            }
            csv_one = root / "one.csv"
            csv_two = root / "two.csv"
            baseline.write_summary_csv(csv_one, [aggregate], [])
            baseline.write_summary_csv(csv_two, [aggregate], [])
            self.assertEqual(csv_one.read_bytes(), csv_two.read_bytes())
            self.assertEqual(csv_one.read_text().splitlines()[0], ",".join(baseline.CSV_COLUMNS))

    def test_valid_v1_benchmark_artifact_renders_deterministic_reports(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(root, run_id="report-source")
            populate_benchmark_evidence(run)
            run.finalize()

            self.assertEqual(baseline.validate_artifact(root, run.run_dir), [])
            integrated_json = run.run_dir / baseline.REPORT_JSON_NAME
            integrated_markdown = run.run_dir / baseline.REPORT_MARKDOWN_NAME
            self.assertTrue(integrated_json.is_file())
            self.assertTrue(integrated_markdown.is_file())
            report = baseline.build_benchmark_report(root, run.run_dir)
            self.assertEqual(integrated_json.read_text(), baseline.report_json_text(report))
            self.assertEqual(
                integrated_markdown.read_text(), baseline.render_report_markdown(report)
            )
            self.assertEqual(report["report_schema"], {"name": "benchmark-report", "version": 1})
            self.assertEqual(report["run"]["status"], "valid")
            self.assertEqual(
                report["evidence"],
                {
                    "artifact_validity": "valid",
                    "budget_decision": "not_evaluated",
                    "classification": "observational_only",
                    "performance_comparability": "not_proven",
                    "runner_isolation": "not_proven",
                    "statistical_significance": "not_evaluated",
                },
            )
            self.assertIn(
                baseline.CRITERION_PRODUCER_ID,
                {item["id"] for item in report["producers"]},
            )
            self.assertTrue(report["correctness"]["zero_errors"])
            self.assertTrue(report["correctness"]["zero_retries"])

            integrated_json_bytes = integrated_json.read_bytes()
            integrated_markdown_bytes = integrated_markdown.read_bytes()
            integrated_json.unlink()
            integrated_markdown.unlink()
            baseline.write_checksums(root, run.run_dir)
            self.assertEqual(baseline.validate_artifact(root, run.run_dir), [])
            report = baseline.build_benchmark_report(root, run.run_dir)

            checksum_before = (run.run_dir / "checksums.sha256").read_bytes()
            first_json, first_markdown = baseline.render_report_to_directory(
                root, run.run_dir, "rendered/first"
            )
            second_json, second_markdown = baseline.render_report_to_directory(
                root, run.run_dir, "rendered/second"
            )
            self.assertEqual(first_json.read_bytes(), second_json.read_bytes())
            self.assertEqual(first_markdown.read_bytes(), second_markdown.read_bytes())
            self.assertEqual(first_json.read_bytes(), integrated_json_bytes)
            self.assertEqual(first_markdown.read_bytes(), integrated_markdown_bytes)
            self.assertEqual(
                (run.run_dir / "checksums.sha256").read_bytes(), checksum_before
            )
            self.assertEqual(baseline.validate_report_document(report), [])

    def test_comparison_records_only_deterministic_signed_observations(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(root, run_id="comparison-baseline")
            populate_benchmark_evidence(run)
            run.finalize()
            baseline_report = baseline.build_benchmark_report(root, run.run_dir)
            candidate_report = shifted_report_fixture(baseline_report)

            comparison = baseline.build_benchmark_comparison(
                baseline_report, candidate_report
            )
            self.assertEqual(
                comparison["comparison_schema"],
                {"name": "benchmark-comparison", "version": 1},
            )
            self.assertEqual(
                comparison["input_report_schema"],
                {"name": "benchmark-report", "version": 1},
            )
            self.assertEqual(comparison["evidence"], baseline.COMPARISON_EVIDENCE)
            self.assertEqual(
                comparison["operands"]["baseline"]["runner"],
                baseline_report["runner"],
            )
            self.assertEqual(
                comparison["operands"]["candidate"]["runner"],
                candidate_report["runner"],
            )
            stress = next(
                scenario
                for scenario in comparison["scenarios"]
                if scenario["kind"] == "tcp_stress"
                and scenario["identity"]["operation"] == "read"
                and scenario["identity"]["in_flight"] == 1
            )
            self.assertEqual(
                stress["observations"]["throughput"]["candidate_minus_baseline"],
                25.0,
            )
            self.assertEqual(
                stress["observations"]["throughput"]["unit"],
                "operations_per_second",
            )
            self.assertAlmostEqual(
                stress["observations"]["p99_latency"]["candidate_minus_baseline"],
                -0.05,
            )
            self.assertEqual(stress["observations"]["p99_latency"]["unit"], "ms")
            criterion = next(
                scenario
                for scenario in comparison["scenarios"]
                if scenario["kind"] == "criterion_estimate"
            )
            self.assertEqual(
                criterion["observations"]["mean_estimate"][
                    "candidate_minus_baseline"
                ],
                4.0,
            )
            self.assertEqual(criterion["observations"]["mean_estimate"]["unit"], "ns")
            self.assertEqual(baseline.validate_comparison_document(comparison), [])

            first = baseline.comparison_json_text(comparison)
            self.assertEqual(first, baseline.comparison_json_text(comparison))
            self.assertTrue(first.endswith("\n"))
            permuted_baseline = copy.deepcopy(baseline_report)
            permuted_candidate = copy.deepcopy(candidate_report)
            permuted_baseline["scenarios"].reverse()
            permuted_candidate["scenarios"] = (
                permuted_candidate["scenarios"][2:]
                + permuted_candidate["scenarios"][:2]
            )
            self.assertEqual(
                first,
                baseline.comparison_json_text(
                    baseline.build_benchmark_comparison(
                        permuted_baseline, permuted_candidate
                    )
                ),
            )

            reversed_comparison = baseline.build_benchmark_comparison(
                candidate_report, baseline_report
            )
            for forward, reversed_observation in zip(
                comparison["scenarios"], reversed_comparison["scenarios"], strict=True
            ):
                self.assertEqual(forward["identity"], reversed_observation["identity"])
                self.assertEqual(forward["kind"], reversed_observation["kind"])
                for metric_name, observation in forward["observations"].items():
                    reversed_metric = reversed_observation["observations"][metric_name]
                    self.assertEqual(observation["unit"], reversed_metric["unit"])
                    self.assertEqual(observation["baseline"], reversed_metric["candidate"])
                    self.assertEqual(observation["candidate"], reversed_metric["baseline"])
                    self.assertEqual(
                        observation["candidate_minus_baseline"],
                        -reversed_metric["candidate_minus_baseline"],
                    )

    def test_comparison_rejects_incompatible_reports_and_incomplete_sets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(root, run_id="comparison-mismatches")
            populate_benchmark_evidence(run)
            run.finalize()
            baseline_report = baseline.build_benchmark_report(root, run.run_dir)

            cases = {
                "schema": lambda value: value["report_schema"].update({"version": 2}),
                "mode": lambda value: value["run"].update({"mode": "bench-full"}),
                "producer": lambda value: value["producers"][-1].update(
                    {"version": "0.5.2"}
                ),
                "scenario-key": lambda value: next(
                    scenario
                    for scenario in value["scenarios"]
                    if scenario["kind"] == "tcp_stress"
                )["identity"].update({"clients": 2}),
                "unit": lambda value: next(
                    scenario
                    for scenario in value["scenarios"]
                    if scenario["kind"] == "tcp_stress"
                )["metrics"]["throughput"].update({"unit": "requests_per_second"}),
                "metric-shape": lambda value: next(
                    scenario
                    for scenario in value["scenarios"]
                    if scenario["kind"] == "tcp_stress"
                )["metrics"]["throughput"]["recorded_statistics"].pop("mean"),
            }
            for label, mutate in cases.items():
                with self.subTest(case=label):
                    candidate = shifted_report_fixture(baseline_report)
                    mutate(candidate)
                    with self.assertRaises(baseline.BaselineError):
                        baseline.build_benchmark_comparison(
                            baseline_report, candidate
                        )

            confidence_mismatch = shifted_report_fixture(baseline_report)
            criterion = next(
                scenario
                for scenario in confidence_mismatch["scenarios"]
                if scenario["kind"] == "criterion_estimate"
            )
            criterion["metrics"]["mean_estimate"]["confidence_level"] = 0.9
            with self.assertRaisesRegex(baseline.BaselineError, "confidence levels"):
                baseline.build_benchmark_comparison(
                    baseline_report, confidence_mismatch
                )

            missing = shifted_report_fixture(baseline_report)
            missing_index = next(
                index
                for index, scenario in enumerate(missing["scenarios"])
                if scenario["kind"] == "tcp_stress"
            )
            removed = missing["scenarios"].pop(missing_index)
            missing["correctness"]["stress_sample_count"] -= removed["identity"][
                "repetitions"
            ]
            self.assertEqual(baseline.validate_report_document(missing), [])
            with self.assertRaisesRegex(baseline.BaselineError, "scenario sets"):
                baseline.build_benchmark_comparison(baseline_report, missing)

            extra = shifted_report_fixture(baseline_report)
            extra_stress = copy.deepcopy(
                next(
                    scenario
                    for scenario in extra["scenarios"]
                    if scenario["kind"] == "tcp_stress"
                )
            )
            extra_stress["identity"]["clients"] = 2
            extra["scenarios"].append(extra_stress)
            extra["correctness"]["stress_sample_count"] += extra_stress["identity"][
                "repetitions"
            ]
            self.assertEqual(baseline.validate_report_document(extra), [])
            with self.assertRaisesRegex(baseline.BaselineError, "scenario sets"):
                baseline.build_benchmark_comparison(baseline_report, extra)

            duplicate_criterion = shifted_report_fixture(baseline_report)
            duplicate = copy.deepcopy(
                next(
                    scenario
                    for scenario in duplicate_criterion["scenarios"]
                    if scenario["kind"] == "criterion_estimate"
                )
            )
            duplicate["sources"][0]["private_estimates_json"] = (
                "criterion/raw/02-tcp-throughput/tcp_pipelined/new/estimates.json"
            )
            duplicate_criterion["scenarios"].append(duplicate)
            self.assertEqual(
                baseline.validate_report_document(duplicate_criterion), []
            )
            with self.assertRaisesRegex(baseline.BaselineError, "duplicate criterion"):
                baseline.build_benchmark_comparison(
                    baseline_report, duplicate_criterion
                )

    def test_comparison_rejects_hostile_selectors_and_numbers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(root, run_id="comparison-hostile")
            populate_benchmark_evidence(run)
            run.finalize()
            baseline_report = baseline.build_benchmark_report(root, run.run_dir)
            candidate_report = shifted_report_fixture(baseline_report)

            for value_label, invalid_value in (
                ("list", ["tcp_stress"]),
                ("object", {"kind": "tcp_stress"}),
            ):
                with self.subTest(selector=value_label):
                    malformed = copy.deepcopy(candidate_report)
                    malformed["scenarios"][0]["kind"] = invalid_value
                    with self.assertRaises(baseline.BaselineError) as raised:
                        baseline.build_benchmark_comparison(
                            baseline_report, malformed
                        )
                    self.assertNotIsInstance(raised.exception, (TypeError, KeyError))

            numeric_cases = {
                "boolean-report-value": True,
                "infinite-report-value": math.inf,
                "nan-report-value": math.nan,
            }
            for label, invalid_value in numeric_cases.items():
                with self.subTest(case=label):
                    malformed = copy.deepcopy(candidate_report)
                    stress = next(
                        scenario
                        for scenario in malformed["scenarios"]
                        if scenario["kind"] == "tcp_stress"
                    )
                    stress["metrics"]["throughput"]["recorded_statistics"][
                        "mean"
                    ] = invalid_value
                    with self.assertRaises(baseline.BaselineError):
                        baseline.build_benchmark_comparison(
                            baseline_report, malformed
                        )

            comparison = baseline.build_benchmark_comparison(
                baseline_report, candidate_report
            )
            stress_index = next(
                index
                for index, scenario in enumerate(comparison["scenarios"])
                if scenario["kind"] == "tcp_stress"
            )
            comparison_cases = {
                "boolean": ("baseline", True),
                "infinite": ("candidate", math.inf),
                "nan-delta": ("candidate_minus_baseline", math.nan),
                "wrong-delta": ("candidate_minus_baseline", 0.0),
            }
            for label, (field, invalid_value) in comparison_cases.items():
                with self.subTest(comparison=label):
                    malformed = copy.deepcopy(comparison)
                    malformed["scenarios"][stress_index]["observations"][
                        "throughput"
                    ][field] = invalid_value
                    self.assertTrue(
                        baseline.validate_comparison_document(malformed)
                    )
                    with self.assertRaises(baseline.BaselineError):
                        baseline.comparison_json_text(malformed)

            for label, path in (
                ("comparison-schema", ("comparison_schema", "version")),
                (
                    "source-schema",
                    (
                        "operands",
                        "baseline",
                        "source_artifact",
                        "schema",
                        "version",
                    ),
                ),
            ):
                with self.subTest(boolean_version=label):
                    malformed = copy.deepcopy(comparison)
                    target = malformed
                    for part in path[:-1]:
                        target = target[part]
                    target[path[-1]] = True
                    self.assertTrue(
                        baseline.validate_comparison_document(malformed)
                    )

    def test_compare_report_cli_is_json_only_fail_closed_and_read_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(root, run_id="comparison-cli")
            populate_benchmark_evidence(run)
            run.finalize()
            baseline_report = baseline.build_benchmark_report(root, run.run_dir)
            candidate_report = shifted_report_fixture(baseline_report)
            report_dir = root / "reports"
            report_dir.mkdir()
            baseline_path = report_dir / "baseline.json"
            candidate_path = report_dir / "candidate.json"
            baseline.write_json(baseline_path, baseline_report)
            baseline.write_json(candidate_path, candidate_report)
            before = (baseline_path.read_bytes(), candidate_path.read_bytes())

            direct = baseline.compare_report_files(
                root, "reports/baseline.json", "reports/candidate.json"
            )
            script = root / "scripts" / "baseline.py"
            script.parent.mkdir()
            shutil.copyfile(Path(baseline.__file__), script)
            success = subprocess.run(
                (
                    sys.executable,
                    str(script),
                    "compare-report",
                    "reports/baseline.json",
                    "reports/candidate.json",
                ),
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(success.returncode, 0, success.stderr)
            self.assertEqual(success.stderr, "")
            self.assertEqual(success.stdout, baseline.comparison_json_text(direct))
            self.assertEqual(json.loads(success.stdout), direct)

            invalid_path = report_dir / "invalid.json"
            invalid_path.write_text("[]\n")
            failure = subprocess.run(
                (
                    sys.executable,
                    str(script),
                    "compare-report",
                    "reports/invalid.json",
                    "reports/candidate.json",
                ),
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(failure.returncode, 0)
            self.assertEqual(failure.stdout, "")
            self.assertIn("baseline:", failure.stderr)
            self.assertEqual(
                (baseline_path.read_bytes(), candidate_path.read_bytes()), before
            )

            link = report_dir / "baseline-link.json"
            try:
                link.symlink_to(baseline_path.name)
            except OSError:
                return
            with self.assertRaisesRegex(baseline.BaselineError, "symlinks"):
                baseline.compare_report_files(
                    root,
                    "reports/baseline-link.json",
                    "reports/candidate.json",
                )

    def test_controlled_evidence_contract_is_valid_canonical_and_permutation_stable(
        self,
    ) -> None:
        document = controlled_evidence_contract_fixture()
        snapshot = copy.deepcopy(document)
        self.assertEqual(baseline.validate_controlled_evidence_contract(document), [])
        rendered = baseline.controlled_evidence_contract_json_text(document)
        digest = baseline.controlled_evidence_contract_sha256(document)
        self.assertTrue(rendered.endswith("\n"))
        self.assertEqual(len(digest), 64)
        self.assertEqual(document, snapshot)

        permuted = copy.deepcopy(document)
        permuted["variance_studies"].reverse()
        for study in permuted["variance_studies"]:
            study["runs"].reverse()
        permuted["baselines"].reverse()
        permuted["budget_rules"].reverse()
        permuted["evidence_retention"].reverse()
        permuted["control"]["runner"]["evidence_ids"].reverse()
        self.assertEqual(baseline.validate_controlled_evidence_contract(permuted), [])
        self.assertEqual(
            baseline.controlled_evidence_contract_json_text(permuted), rendered
        )
        self.assertEqual(baseline.controlled_evidence_contract_sha256(permuted), digest)

        canonical = json.loads(rendered)
        self.assertEqual(
            [study["study_id"] for study in canonical["variance_studies"]],
            sorted(study["study_id"] for study in canonical["variance_studies"]),
        )
        self.assertEqual(
            canonical["control"]["runner"]["evidence_ids"],
            sorted(canonical["control"]["runner"]["evidence_ids"]),
        )
        self.assertFalse(
            {"verdict", "significance", "calculation", "attestation", "environment"}
            & set(rendered.split('"'))
        )

        changed_approval_record = copy.deepcopy(document)
        changed_approval_record["approval"]["approved_utc"] = "2026-03-01T00:00:00Z"
        self.assertEqual(
            baseline.validate_controlled_evidence_contract(changed_approval_record), []
        )
        self.assertEqual(
            baseline._controlled_evidence_contract_approval_scope_sha256(
                changed_approval_record
            ),
            baseline._controlled_evidence_contract_approval_scope_sha256(document),
        )
        self.assertNotEqual(
            baseline.controlled_evidence_contract_sha256(changed_approval_record), digest
        )

    def test_controlled_evidence_contract_rejects_unknown_missing_or_unsupported_schema(
        self,
    ) -> None:
        cases = []
        missing = controlled_evidence_contract_fixture()
        missing.pop("approval_authority")
        cases.append(missing)
        extra = controlled_evidence_contract_fixture()
        extra["performance_verdict"] = "pass"
        cases.append(extra)
        unknown_name = controlled_evidence_contract_fixture()
        unknown_name["contract_schema"]["name"] = "unknown-contract"
        cases.append(unknown_name)
        unknown_version = controlled_evidence_contract_fixture()
        unknown_version["contract_schema"]["version"] = 2
        cases.append(unknown_version)
        boolean_version = controlled_evidence_contract_fixture()
        boolean_version["contract_schema"]["version"] = True
        cases.append(boolean_version)
        alpha = controlled_evidence_contract_fixture()
        alpha["statistical_method"]["alpha"] = 0.05
        cases.append(alpha)
        significance = controlled_evidence_contract_fixture()
        significance["statistical_method"]["outputs"]["significance"] = "required"
        cases.append(significance)
        unknown_state = controlled_evidence_contract_fixture()
        unknown_state["baselines"][0]["state"] = "accepted"
        cases.append(unknown_state)
        for position, malformed in enumerate(cases):
            with self.subTest(case=position):
                self.assert_controlled_contract_invalid(malformed)

    def test_controlled_evidence_contract_rejects_duplicates_and_unresolved_references(
        self,
    ) -> None:
        cases = []
        duplicate_evidence = controlled_evidence_contract_fixture()
        duplicate_evidence["evidence_retention"].append(
            copy.deepcopy(duplicate_evidence["evidence_retention"][0])
        )
        cases.append(duplicate_evidence)
        duplicate_study = controlled_evidence_contract_fixture()
        duplicate_study["variance_studies"].append(
            copy.deepcopy(duplicate_study["variance_studies"][0])
        )
        cases.append(duplicate_study)
        duplicate_baseline = controlled_evidence_contract_fixture()
        duplicate_baseline["baselines"][1]["baseline_id"] = duplicate_baseline[
            "baselines"
        ][0]["baseline_id"]
        cases.append(duplicate_baseline)
        duplicate_run = controlled_evidence_contract_fixture()
        duplicate_run["variance_studies"][1]["runs"].append(
            copy.deepcopy(duplicate_run["variance_studies"][0]["runs"][0])
        )
        cases.append(duplicate_run)
        duplicate_binding = controlled_evidence_contract_fixture()
        duplicate_binding["control"]["binding"]["evidence_ids"].append(
            duplicate_binding["control"]["binding"]["evidence_ids"][0]
        )
        cases.append(duplicate_binding)
        unresolved_runner = controlled_evidence_contract_fixture()
        unresolved_runner["control"]["runner"]["evidence_ids"][0] = "missing-evidence"
        cases.append(unresolved_runner)
        unresolved_artifact = controlled_evidence_contract_fixture()
        unresolved_artifact["variance_studies"][0]["runs"][0][
            "artifact_evidence_id"
        ] = "missing-evidence"
        cases.append(unresolved_artifact)
        unresolved_successor = controlled_evidence_contract_fixture()
        unresolved_successor["baselines"][0]["supersession"][
            "successor_baseline_id"
        ] = "missing-baseline"
        cases.append(unresolved_successor)
        for position, malformed in enumerate(cases):
            with self.subTest(case=position):
                self.assert_controlled_contract_invalid(malformed)

    def test_controlled_evidence_contract_rejects_malformed_identity_and_retention(
        self,
    ) -> None:
        cases = []
        malformed_sha = controlled_evidence_contract_fixture()
        malformed_sha["variance_studies"][0]["target_sha"] = "abc"
        cases.append(malformed_sha)
        malformed_digest = controlled_evidence_contract_fixture()
        malformed_digest["evidence_retention"][0]["sha256"] = "not-a-digest"
        cases.append(malformed_digest)
        uppercase_digest = controlled_evidence_contract_fixture()
        uppercase_digest["variance_studies"][0]["producer_set_sha256"] = "A" * 64
        cases.append(uppercase_digest)
        malformed_timestamp = controlled_evidence_contract_fixture()
        malformed_timestamp["evidence_retention"][0]["recorded_utc"] = (
            "2026-01-01T00:00:00+00:00"
        )
        cases.append(malformed_timestamp)
        reversed_retention = controlled_evidence_contract_fixture()
        reversed_retention["evidence_retention"][0]["retained_until_utc"] = (
            "2025-01-01T00:00:00Z"
        )
        cases.append(reversed_retention)
        malformed_locator = controlled_evidence_contract_fixture()
        malformed_locator["evidence_retention"][0]["locator"] = "opaque:has space"
        cases.append(malformed_locator)
        oversized_locator = controlled_evidence_contract_fixture()
        oversized_locator["evidence_retention"][0]["locator"] = "x" * 513
        cases.append(oversized_locator)
        malformed_scope = controlled_evidence_contract_fixture()
        malformed_scope["approval"]["scope_sha256"] = "0" * 63
        cases.append(malformed_scope)
        for position, malformed in enumerate(cases):
            with self.subTest(case=position):
                self.assert_controlled_contract_invalid(malformed)

    def test_controlled_evidence_contract_rejects_variance_coherence_mismatches(
        self,
    ) -> None:
        cases = []
        insufficient = controlled_evidence_contract_fixture()
        insufficient["variance_studies"][0]["runs"].pop()
        cases.append(insufficient)
        boolean_minimum = controlled_evidence_contract_fixture()
        boolean_minimum["statistical_method"]["minimum_independent_runs"] = True
        cases.append(boolean_minimum)
        profile_mismatch = controlled_evidence_contract_fixture()
        profile_mismatch["variance_studies"][0]["runs"][0]["control_profile"][
            "profile_id"
        ] = "synthetic-other-profile"
        cases.append(profile_mismatch)
        method_mismatch = controlled_evidence_contract_fixture()
        method_mismatch["variance_studies"][0]["runs"][0]["statistical_method"][
            "version"
        ] = "synthetic-v2"
        cases.append(method_mismatch)
        target_mismatch = controlled_evidence_contract_fixture()
        target_mismatch["variance_studies"][0]["runs"][0]["target_sha"] = "c" * 40
        cases.append(target_mismatch)
        scenario_mismatch = controlled_evidence_contract_fixture()
        scenario_mismatch["variance_studies"][0]["runs"][0][
            "scenario_set_sha256"
        ] = "7" * 64
        cases.append(scenario_mismatch)
        producer_mismatch = controlled_evidence_contract_fixture()
        producer_mismatch["variance_studies"][0]["runs"][0][
            "producer_set_sha256"
        ] = "8" * 64
        cases.append(producer_mismatch)
        incomplete_sample = controlled_evidence_contract_fixture()
        incomplete_sample["variance_studies"][0]["runs"][0]["sample_unit"] = (
            "partial_bench-full_run"
        )
        cases.append(incomplete_sample)
        partial_alignment = controlled_evidence_contract_fixture()
        partial_alignment["statistical_method"]["scenario_alignment"] = "intersection"
        cases.append(partial_alignment)
        for position, malformed in enumerate(cases):
            with self.subTest(case=position):
                self.assert_controlled_contract_invalid(malformed)

    def test_controlled_evidence_contract_requires_distinct_variance_artifacts(
        self,
    ) -> None:
        reused_evidence = controlled_evidence_contract_fixture()
        first_run, second_run = reused_evidence["variance_studies"][0]["runs"]
        second_run["artifact_evidence_id"] = first_run["artifact_evidence_id"]
        errors = baseline.validate_controlled_evidence_contract(reused_evidence)
        self.assertIn("distinct benchmark artifact evidence IDs", errors[0])
        self.assert_controlled_contract_invalid(reused_evidence)

        reused_content = controlled_evidence_contract_fixture()
        evidence_by_id = {
            evidence["evidence_id"]: evidence
            for evidence in reused_content["evidence_retention"]
        }
        evidence_by_id["evidence-run-old-b"]["sha256"] = evidence_by_id[
            "evidence-run-old-a"
        ]["sha256"]
        errors = baseline.validate_controlled_evidence_contract(reused_content)
        self.assertIn("distinct benchmark artifact content digests", errors[0])
        self.assert_controlled_contract_invalid(reused_content)

    def test_controlled_evidence_contract_lifecycle_prefixes_and_approval_are_strict(
        self,
    ) -> None:
        for state, prefix_length in (
            ("candidate", 0),
            ("variance_collected", 1),
            ("promotion_pending", 2),
        ):
            with self.subTest(valid_prefix=state):
                partial = controlled_evidence_contract_fixture()
                record = partial["baselines"][1]
                record["state"] = state
                record["promotion_chain"] = record["promotion_chain"][:prefix_length]
                record["supersession"] = None
                partial["baselines"] = [record]
                partial["approval"] = None
                self.assertEqual(
                    baseline.validate_controlled_evidence_contract(partial), []
                )

        cases = []
        bad_initial = controlled_evidence_contract_fixture()
        bad_initial["baseline_lifecycle"]["initial_state"] = "approved"
        cases.append(bad_initial)
        direct_lifecycle_path = controlled_evidence_contract_fixture()
        direct_lifecycle_path["baseline_lifecycle"]["promotion_path"] = [
            {"from_state": "candidate", "to_state": "approved"}
        ]
        cases.append(direct_lifecycle_path)
        direct_approval = controlled_evidence_contract_fixture()
        direct_approval["baselines"][1]["promotion_chain"] = [
            {
                "approval_id": direct_approval["approval"]["approval_id"],
                "evidence_id": direct_approval["approval"]["evidence_id"],
                "from_state": "candidate",
                "to_state": "approved",
            }
        ]
        cases.append(direct_approval)
        omitted_stage = controlled_evidence_contract_fixture()
        omitted_stage["baselines"][1]["promotion_chain"].pop(1)
        cases.append(omitted_stage)
        reordered_stages = controlled_evidence_contract_fixture()
        reordered_stages["baselines"][1]["promotion_chain"][0:2] = reversed(
            reordered_stages["baselines"][1]["promotion_chain"][0:2]
        )
        cases.append(reordered_stages)
        skipped_stage = controlled_evidence_contract_fixture()
        skipped_stage["baselines"][1]["promotion_chain"][1].update(
            {"from_state": "candidate", "to_state": "promotion_pending"}
        )
        cases.append(skipped_stage)
        wrong_partial_prefix = controlled_evidence_contract_fixture()
        wrong_partial = wrong_partial_prefix["baselines"][1]
        wrong_partial["state"] = "promotion_pending"
        wrong_partial["promotion_chain"] = wrong_partial["promotion_chain"][1:]
        wrong_partial_prefix["baselines"] = [wrong_partial]
        cases.append(wrong_partial_prefix)
        early_approval = controlled_evidence_contract_fixture()
        early_record = early_approval["baselines"][1]
        early_record["state"] = "promotion_pending"
        early_record["promotion_chain"] = early_record["promotion_chain"][:2]
        early_record["promotion_chain"][1]["approval_id"] = early_approval["approval"][
            "approval_id"
        ]
        early_approval["baselines"] = [early_record]
        cases.append(early_approval)
        missing_approval = controlled_evidence_contract_fixture()
        missing_approval["approval"] = None
        cases.append(missing_approval)
        authority_mismatch = controlled_evidence_contract_fixture()
        authority_mismatch["approval"]["authority_id"] = "synthetic-other-authority"
        cases.append(authority_mismatch)
        scope_mismatch = controlled_evidence_contract_fixture()
        scope_mismatch["approval"]["scope_sha256"] = "f" * 64
        cases.append(scope_mismatch)
        unresolved_study = controlled_evidence_contract_fixture()
        unresolved_study["baselines"][1]["promotion_chain"][0][
            "variance_study_id"
        ] = "missing-study"
        cases.append(unresolved_study)
        variance_evidence_mismatch = controlled_evidence_contract_fixture()
        variance_evidence_mismatch["baselines"][1]["promotion_chain"][0][
            "evidence_id"
        ] = "evidence-variance-old"
        cases.append(variance_evidence_mismatch)
        rule_evidence_mismatch = controlled_evidence_contract_fixture()
        rule_evidence_mismatch["baselines"][1]["promotion_chain"][1][
            "rule_evidence_id"
        ] = "evidence-promotion-old"
        cases.append(rule_evidence_mismatch)
        approval_evidence_mismatch = controlled_evidence_contract_fixture()
        approval_evidence_mismatch["baselines"][1]["promotion_chain"][2][
            "evidence_id"
        ] = "evidence-promotion-current"
        cases.append(approval_evidence_mismatch)
        approved_with_successor = controlled_evidence_contract_fixture()
        approved_with_successor["baselines"][1]["supersession"] = copy.deepcopy(
            approved_with_successor["baselines"][0]["supersession"]
        )
        cases.append(approved_with_successor)
        successor_not_approved = controlled_evidence_contract_fixture()
        successor_not_approved["baselines"][1]["state"] = "candidate"
        successor_not_approved["baselines"][1]["promotion_chain"] = []
        cases.append(successor_not_approved)
        bad_supersession = controlled_evidence_contract_fixture()
        bad_supersession["baselines"][0]["supersession"]["from_state"] = "candidate"
        cases.append(bad_supersession)
        for position, malformed in enumerate(cases):
            with self.subTest(case=position):
                self.assert_controlled_contract_invalid(malformed)

    def test_controlled_evidence_contract_rejects_budget_selectors_pairings_and_limits(
        self,
    ) -> None:
        cases = []
        wildcard = controlled_evidence_contract_fixture()
        wildcard["budget_rules"][0]["scenario_identity"]["scenario_id"] = "*"
        cases.append(wildcard)
        partial_match = controlled_evidence_contract_fixture()
        partial_match["budget_rules"][0]["scenario_identity"]["match"] = "prefix"
        cases.append(partial_match)
        partial_selector = controlled_evidence_contract_fixture()
        partial_selector["budget_rules"][0]["scenario_identity"].pop("identity_sha256")
        cases.append(partial_selector)
        wrong_metric = controlled_evidence_contract_fixture()
        wrong_metric["budget_rules"][0]["metric"] = "median"
        cases.append(wrong_metric)
        wrong_unit = controlled_evidence_contract_fixture()
        wrong_unit["budget_rules"][0]["unit"] = "ms"
        cases.append(wrong_unit)
        wrong_direction = controlled_evidence_contract_fixture()
        wrong_direction["budget_rules"][0]["direction"] = "maximum"
        cases.append(wrong_direction)
        for invalid_limit in (True, -1, math.nan, math.inf, -math.inf):
            invalid = controlled_evidence_contract_fixture()
            invalid["budget_rules"][0]["limit"] = invalid_limit
            cases.append(invalid)
        verdict = controlled_evidence_contract_fixture()
        verdict["budget_rules"][0]["verdict"] = "pass"
        cases.append(verdict)
        calculation = controlled_evidence_contract_fixture()
        calculation["budget_rules"][0]["calculation"] = "candidate-baseline"
        cases.append(calculation)
        duplicate_id = controlled_evidence_contract_fixture()
        duplicate_id["budget_rules"][1]["budget_rule_id"] = duplicate_id[
            "budget_rules"
        ][0]["budget_rule_id"]
        cases.append(duplicate_id)
        for position, malformed in enumerate(cases):
            with self.subTest(case=position):
                self.assert_controlled_contract_invalid(malformed)

    def test_controlled_evidence_integrity_labels_and_locators_confer_no_authority(
        self,
    ) -> None:
        cases = []
        signature_claim = controlled_evidence_contract_fixture()
        signature_claim["semantics"]["integrity"] = "sha256_is_a_signature"
        cases.append(signature_claim)
        runner_label = controlled_evidence_contract_fixture()
        runner_label["control"]["runner"]["label"] = "controlled"
        cases.append(runner_label)
        no_profile_evidence = controlled_evidence_contract_fixture()
        no_profile_evidence["control"]["profile"]["evidence_ids"] = []
        cases.append(no_profile_evidence)
        digest_as_binding = controlled_evidence_contract_fixture()
        digest_as_binding["control"]["binding"]["evidence_ids"] = ["a" * 64]
        cases.append(digest_as_binding)
        locator_as_authority = controlled_evidence_contract_fixture()
        locator_as_authority["approval_authority"]["evidence_id"] = (
            "opaque:synthetic/authority"
        )
        cases.append(locator_as_authority)
        attestation_field = controlled_evidence_contract_fixture()
        attestation_field["evidence_retention"][0]["attestation"] = True
        cases.append(attestation_field)
        for position, malformed in enumerate(cases):
            with self.subTest(case=position):
                self.assert_controlled_contract_invalid(malformed)

        parser = baseline.build_parser()
        self.assertNotIn("controlled-evidence-contract", parser.format_help())
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(["validate-controlled-evidence-contract", "contract.json"])

    def test_checked_in_disabled_policy_is_strict_and_canonical(self) -> None:
        relative = "benchmarks/policy/benchmark-budget-policy-v1.json"
        policy = baseline.load_policy_file(ROOT, relative)
        self.assertEqual(baseline.validate_policy_document(policy), [])
        self.assertEqual((ROOT / relative).read_text(), baseline.policy_json_text(policy))

        reversed_blockers = copy.deepcopy(policy)
        reversed_blockers["activation_blockers"].reverse()
        self.assertEqual(baseline.validate_policy_document(reversed_blockers), [])
        self.assertEqual(
            baseline.policy_canonical_sha256(reversed_blockers),
            baseline.policy_canonical_sha256(policy),
        )

        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(baseline.main(["validate-policy", relative]), 0)
        self.assertEqual(stdout.getvalue(), f"benchmark policy valid: {relative}\n")

    def test_disabled_policy_rejects_unknown_or_active_content(self) -> None:
        policy = disabled_policy_fixture()
        cases: dict[str, tuple[tuple[object, ...], object]] = {
            "unknown-schema": (("policy_schema", "name"), "unknown-policy"),
            "unknown-version": (("policy_schema", "version"), 2),
            "boolean-version": (("policy_schema", "version"), True),
            "unknown-state": (("policy_state",), "active"),
            "malformed-id": (("policy_id",), "Owner Approval"),
            "duplicate-blocker": (
                ("activation_blockers",),
                [*baseline.POLICY_ACTIVATION_BLOCKERS, baseline.POLICY_ACTIVATION_BLOCKERS[0]],
            ),
            "missing-blocker": (
                ("activation_blockers",),
                list(baseline.POLICY_ACTIVATION_BLOCKERS[:-1]),
            ),
            "approved-baseline": (("approved_baseline",), {"target_sha": "a" * 40}),
            "control-profile": (("control_profile",), {"runner": "controlled"}),
            "variance-evidence": (("variance_evidence",), [{"digest": "a" * 64}]),
            "duplicate-evidence": (
                ("variance_evidence",),
                [{"digest": "a" * 64}, {"digest": "a" * 64}],
            ),
            "budget-rule": (("budget_rules",), [{"metric": "throughput"}]),
            "threshold": (("thresholds",), [5.0]),
            "non-finite-threshold": (("thresholds",), [math.inf]),
            "statistical-method": (("statistical_method",), "unsupported"),
            "approval": (("approval",), {"owner": "not-authority"}),
        }
        for label, (path, invalid_value) in cases.items():
            with self.subTest(case=label):
                malformed = copy.deepcopy(policy)
                target = malformed
                for part in path[:-1]:
                    target = target[part]
                target[path[-1]] = invalid_value
                self.assertTrue(baseline.validate_policy_document(malformed))
                with self.assertRaises(baseline.BaselineError):
                    baseline.policy_json_text(malformed)

        missing = copy.deepcopy(policy)
        missing.pop("approval")
        self.assertTrue(baseline.validate_policy_document(missing))
        extra = copy.deepcopy(policy)
        extra["owner"] = "not-authority"
        self.assertTrue(baseline.validate_policy_document(extra))

    def test_policy_loader_rejects_duplicate_absolute_traversal_and_symlink_paths(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            policy_path = root / "policy.json"
            policy_path.write_text(baseline.policy_json_text(disabled_policy_fixture()))
            self.assertEqual(baseline.load_policy_file(root, "policy.json"), disabled_policy_fixture())

            with self.assertRaisesRegex(baseline.BaselineError, "repository-relative"):
                baseline.load_policy_file(root, str(policy_path))
            with self.assertRaisesRegex(baseline.BaselineError, "traversal"):
                baseline.load_policy_file(root, "../policy.json")

            duplicate = root / "duplicate.json"
            duplicate.write_text(
                policy_path.read_text().replace(
                    '  "policy_id": "rusty-modbus-benchmark-budget-policy",\n',
                    '  "policy_id": "rusty-modbus-benchmark-budget-policy",\n'
                    '  "policy_id": "duplicate",\n',
                )
            )
            with self.assertRaisesRegex(baseline.BaselineError, "duplicate object key"):
                baseline.load_policy_file(root, "duplicate.json")

            nonfinite = root / "nonfinite.json"
            nonfinite.write_text(policy_path.read_text().replace('"version": 1', '"version": NaN'))
            with self.assertRaisesRegex(baseline.BaselineError, "non-finite"):
                baseline.load_policy_file(root, "nonfinite.json")

            link = root / "policy-link.json"
            try:
                link.symlink_to(policy_path.name)
            except OSError:
                return
            with self.assertRaisesRegex(baseline.BaselineError, "symlinks"):
                baseline.load_policy_file(root, "policy-link.json")

    def test_controlled_evaluate_emits_canonical_not_eligible_document_and_exit(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline_run = self.make_report_run(
                root, run_id="controlled-baseline", mode="bench-full"
            )
            candidate_run = self.make_run(
                root,
                run_id="controlled-candidate",
                target_sha=baseline_run.target_sha,
                mode="bench-full",
            )
            populate_benchmark_evidence(baseline_run)
            populate_benchmark_evidence(candidate_run, measurement_offset=7.0)
            baseline_run.finalize()
            candidate_run.finalize()
            policy_path = root / "policy.json"
            policy_path.write_text(baseline.policy_json_text(disabled_policy_fixture()))

            baseline_relative = baseline_run.run_dir.relative_to(root.resolve()).as_posix()
            candidate_relative = candidate_run.run_dir.relative_to(root.resolve()).as_posix()
            evaluation = baseline.controlled_evaluate_artifacts(
                root, "policy.json", baseline_relative, candidate_relative
            )
            self.assertEqual(baseline.validate_controlled_evaluation_document(evaluation), [])
            self.assertEqual(
                evaluation["performance_enforcement"],
                {
                    "reason_codes": list(baseline.CONTROLLED_EVALUATION_REASON_CODES),
                    "state": "not_eligible",
                },
            )
            self.assertEqual(
                evaluation["observational_comparison"]["evidence"],
                baseline.COMPARISON_EVIDENCE,
            )
            self.assertNotIn("decision", evaluation["performance_enforcement"])
            self.assertNotIn("verdict", evaluation["performance_enforcement"])
            rendered = baseline.controlled_evaluation_json_text(evaluation)
            self.assertEqual(rendered, baseline.controlled_evaluation_json_text(evaluation))

            script = root / "scripts" / "baseline.py"
            script.parent.mkdir()
            shutil.copyfile(Path(baseline.__file__), script)
            before = {
                path.relative_to(root.resolve()).as_posix(): path.read_bytes()
                for run in (baseline_run, candidate_run)
                for path in run.run_dir.rglob("*")
                if path.is_file()
            }
            completed = subprocess.run(
                (
                    sys.executable,
                    str(script),
                    "controlled-evaluate",
                    "policy.json",
                    baseline_relative,
                    candidate_relative,
                ),
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(
                completed.returncode, baseline.CONTROLLED_NOT_ELIGIBLE_EXIT, completed.stderr
            )
            self.assertEqual(completed.stderr, "")
            self.assertEqual(completed.stdout, rendered)
            self.assertEqual(json.loads(completed.stdout), evaluation)
            after = {
                path.relative_to(root.resolve()).as_posix(): path.read_bytes()
                for run in (baseline_run, candidate_run)
                for path in run.run_dir.rglob("*")
                if path.is_file()
            }
            self.assertEqual(after, before)
            self.assertFalse(any(path.name == "latest" for path in root.rglob("*")))

            (candidate_run.run_dir / "summary.csv").write_text("tampered\n")
            invalid = subprocess.run(
                (
                    sys.executable,
                    str(script),
                    "controlled-evaluate",
                    "policy.json",
                    baseline_relative,
                    candidate_relative,
                ),
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(invalid.returncode, 1)
            self.assertEqual(invalid.stdout, "")
            self.assertIn("checksum mismatch", invalid.stderr)

    def test_controlled_evaluation_document_rejects_hostile_identity_and_digest_fields(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline_run = self.make_report_run(
                root, run_id="evaluation-baseline", mode="bench-full"
            )
            candidate_run = self.make_run(
                root,
                run_id="evaluation-candidate",
                target_sha=baseline_run.target_sha,
                mode="bench-full",
            )
            populate_benchmark_evidence(baseline_run)
            populate_benchmark_evidence(candidate_run)
            baseline_run.finalize()
            candidate_run.finalize()
            evaluation = baseline.build_controlled_evaluation(
                disabled_policy_fixture(),
                baseline.build_benchmark_report(root, baseline_run.run_dir),
                baseline.build_benchmark_report(root, candidate_run.run_dir),
            )

            cases = {
                "boolean-version": (
                    ("controlled_evaluation_schema", "version"),
                    True,
                ),
                "unknown-version": (("controlled_evaluation_schema", "version"), 2),
                "malformed-digest": (("policy", "canonical_sha256"), "not-a-digest"),
                "tampered-digest": (("policy", "canonical_sha256"), "b" * 64),
                "boolean-digest": (("policy", "canonical_sha256"), True),
                "malformed-sha": (("operands", "candidate", "target_sha"), "abc"),
                "malformed-run": (("operands", "candidate", "run_id"), "../candidate"),
                "absolute-source": (
                    ("operands", "candidate", "source_artifact"),
                    "/tmp/candidate",
                ),
                "traversal-source": (
                    ("operands", "candidate", "source_artifact"),
                    "bench-output/../candidate",
                ),
                "duplicate-reason": (
                    ("performance_enforcement", "reason_codes"),
                    [
                        *baseline.CONTROLLED_EVALUATION_REASON_CODES,
                        baseline.CONTROLLED_EVALUATION_REASON_CODES[0],
                    ],
                ),
            }
            for label, (path, invalid_value) in cases.items():
                with self.subTest(case=label):
                    malformed = copy.deepcopy(evaluation)
                    target = malformed
                    for part in path[:-1]:
                        target = target[part]
                    target[path[-1]] = invalid_value
                    self.assertTrue(
                        baseline.validate_controlled_evaluation_document(malformed)
                    )
                    with self.assertRaises(baseline.BaselineError):
                        baseline.controlled_evaluation_json_text(malformed)

    def test_controlled_evaluate_rejects_invalid_artifacts_and_unsafe_paths(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline_run = self.make_report_run(
                root, run_id="reject-baseline", mode="bench-full"
            )
            candidate_run = self.make_run(
                root,
                run_id="reject-candidate",
                target_sha=baseline_run.target_sha,
                mode="bench-full",
            )
            populate_benchmark_evidence(baseline_run)
            populate_benchmark_evidence(candidate_run)
            baseline_run.finalize()
            candidate_run.finalize()
            (root / "policy.json").write_text(
                baseline.policy_json_text(disabled_policy_fixture())
            )
            baseline_relative = baseline_run.run_dir.relative_to(root.resolve()).as_posix()
            candidate_relative = candidate_run.run_dir.relative_to(root.resolve()).as_posix()
            snapshot = root / "candidate-snapshot"
            shutil.copytree(candidate_run.run_dir, snapshot)

            def restore_candidate() -> None:
                shutil.rmtree(candidate_run.run_dir)
                shutil.copytree(snapshot, candidate_run.run_dir)

            def evaluate() -> dict:
                return baseline.controlled_evaluate_artifacts(
                    root, "policy.json", baseline_relative, candidate_relative
                )

            (candidate_run.run_dir / "summary.csv").write_text("tampered\n")
            with self.assertRaisesRegex(baseline.BaselineError, "checksum mismatch"):
                evaluate()

            restore_candidate()
            provenance_path = candidate_run.run_dir / "provenance.json"
            summary_path = candidate_run.run_dir / "summary.json"
            provenance = json.loads(provenance_path.read_text())
            summary = json.loads(summary_path.read_text())
            provenance.update(
                {"baseline_eligible": False, "dirty": True, "dirty_override": True}
            )
            summary.update(
                {
                    "baseline_valid": False,
                    "invalid_reasons": ["dirty non-ignored worktree"],
                    "status": "invalid",
                }
            )
            baseline.write_json(provenance_path, provenance)
            baseline.write_json(summary_path, summary)
            baseline.write_checksums(root, candidate_run.run_dir)
            with self.assertRaises(baseline.BaselineError):
                evaluate()

            restore_candidate()
            provenance = json.loads(provenance_path.read_text())
            summary = json.loads(summary_path.read_text())
            provenance["baseline_eligible"] = False
            summary.update(
                {
                    "baseline_valid": False,
                    "invalid_reasons": ["synthetic command failure"],
                    "status": "failed",
                }
            )
            baseline.write_json(provenance_path, provenance)
            baseline.write_json(summary_path, summary)
            baseline.write_checksums(root, candidate_run.run_dir)
            with self.assertRaises(baseline.BaselineError):
                evaluate()

            restore_candidate()
            criterion_source = json.loads(summary_path.read_text())["criterion_results"][0][
                "source"
            ]
            (root / criterion_source).unlink()
            baseline.write_checksums(root, candidate_run.run_dir)
            with self.assertRaisesRegex(baseline.BaselineError, "missing"):
                evaluate()

            restore_candidate()
            with self.assertRaisesRegex(baseline.BaselineError, "repository-relative"):
                baseline.controlled_evaluate_artifacts(
                    root,
                    "policy.json",
                    str(baseline_run.run_dir),
                    candidate_relative,
                )
            with self.assertRaisesRegex(baseline.BaselineError, "traversal"):
                baseline.controlled_evaluate_artifacts(
                    root,
                    "policy.json",
                    baseline_relative,
                    "bench-output/../reject-candidate",
                )
            link = root / "candidate-link"
            try:
                link.symlink_to(
                    candidate_run.run_dir.relative_to(root.resolve()),
                    target_is_directory=True,
                )
            except OSError:
                return
            with self.assertRaisesRegex(baseline.BaselineError, "symlinks"):
                baseline.controlled_evaluate_artifacts(
                    root, "policy.json", baseline_relative, "candidate-link"
                )

    def test_controlled_evaluate_rejects_smoke_same_or_incompatible_operands(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline_run = self.make_report_run(
                root, run_id="compatible-baseline", mode="bench-full"
            )
            candidate_run = self.make_run(
                root,
                run_id="compatible-candidate",
                target_sha=baseline_run.target_sha,
                mode="bench-full",
            )
            smoke_run = self.make_run(
                root,
                run_id="smoke-candidate",
                target_sha=baseline_run.target_sha,
                mode="bench-smoke",
            )
            for run in (baseline_run, candidate_run, smoke_run):
                populate_benchmark_evidence(run)
                run.finalize()
            (root / "policy.json").write_text(
                baseline.policy_json_text(disabled_policy_fixture())
            )
            baseline_relative = baseline_run.run_dir.relative_to(root.resolve()).as_posix()
            candidate_relative = candidate_run.run_dir.relative_to(root.resolve()).as_posix()
            smoke_relative = smoke_run.run_dir.relative_to(root.resolve()).as_posix()

            with self.assertRaisesRegex(baseline.BaselineError, "distinct identities"):
                baseline.controlled_evaluate_artifacts(
                    root, "policy.json", baseline_relative, baseline_relative
                )
            with self.assertRaisesRegex(baseline.BaselineError, "bench-full"):
                baseline.controlled_evaluate_artifacts(
                    root, "policy.json", baseline_relative, smoke_relative
                )

            baseline_report = baseline.build_benchmark_report(root, baseline_run.run_dir)
            candidate_report = baseline.build_benchmark_report(root, candidate_run.run_dir)
            copied_identity = copy.deepcopy(candidate_report)
            copied_identity["run"].update(baseline_report["run"])
            copied_identity["source_artifact"]["path"] = (
                "copied-evidence/"
                + "/".join(baseline_report["source_artifact"]["path"].split("/")[-3:])
            )
            with self.assertRaisesRegex(baseline.BaselineError, "distinct identities"):
                baseline.build_controlled_evaluation(
                    disabled_policy_fixture(), baseline_report, copied_identity
                )

            producer_mismatch = copy.deepcopy(candidate_report)
            producer_mismatch["producers"][-1]["version"] = "unsupported"
            with self.assertRaisesRegex(baseline.BaselineError, "producer"):
                baseline.build_controlled_evaluation(
                    disabled_policy_fixture(), baseline_report, producer_mismatch
                )

            mode_mismatch = copy.deepcopy(candidate_report)
            mode_mismatch["run"]["mode"] = "bench-smoke"
            with self.assertRaisesRegex(baseline.BaselineError, "bench-full"):
                baseline.build_controlled_evaluation(
                    disabled_policy_fixture(), baseline_report, mode_mismatch
                )

            scenario_mismatch = copy.deepcopy(candidate_report)
            removed = scenario_mismatch["scenarios"].pop(0)
            scenario_mismatch["correctness"]["stress_sample_count"] -= removed["identity"][
                "repetitions"
            ]
            with self.assertRaisesRegex(baseline.BaselineError, "scenario sets"):
                baseline.build_controlled_evaluation(
                    disabled_policy_fixture(), baseline_report, scenario_mismatch
                )

            parser = baseline.build_parser()
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                parser.parse_args(["controlled-evaluate", "policy.json"])
            self.assertFalse(any(path.name == "latest" for path in root.rglob("*")))

    def test_target_sha_benchmark_manifest_inventory_is_strict(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            valid_root = parent / "valid"
            valid_root.mkdir()
            valid_sha = initialize_git_lock_fixture(
                valid_root,
                benchmark_targets=("tcp_throughput", "codec", "tcp_pool"),
            )
            self.assertEqual(
                baseline.benchmark_targets_from_target_manifest(valid_root, valid_sha),
                ["codec", "tcp_pool", "tcp_throughput"],
            )
            (valid_root / "benchmarks" / "Cargo.toml").write_text("not valid TOML = [")
            self.assertEqual(
                baseline.benchmark_targets_from_target_manifest(valid_root, valid_sha),
                ["codec", "tcp_pool", "tcp_throughput"],
            )

            cases = (
                (
                    "unavailable",
                    lambda root: initialize_git_lock_fixture(
                        root, include_benchmark_manifest=False
                    ),
                    "unavailable",
                ),
                (
                    "malformed",
                    lambda root: initialize_git_lock_fixture(
                        root,
                        benchmark_manifest_text="[[bench]\nname = 'tcp_throughput'\n",
                    ),
                    "malformed",
                ),
                (
                    "duplicate",
                    lambda root: initialize_git_lock_fixture(
                        root, benchmark_targets=("tcp_pool", "tcp_pool")
                    ),
                    "duplicate",
                ),
                (
                    "empty",
                    lambda root: initialize_git_lock_fixture(root, benchmark_targets=()),
                    r"no \[\[bench\]\] inventory",
                ),
            )
            for label, initialize, expected_error in cases:
                with self.subTest(case=label):
                    root = parent / label
                    root.mkdir()
                    target_sha = initialize(root)
                    with self.assertRaisesRegex(baseline.BaselineError, expected_error):
                        baseline.benchmark_targets_from_target_manifest(root, target_sha)

    def test_controlled_evaluate_rejects_symmetric_missing_criterion_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            targets = ("tcp_pool", "tcp_throughput")
            baseline_run = self.make_report_run(
                root,
                run_id="complete-target-baseline",
                mode="bench-full",
                benchmark_targets=targets,
            )
            candidate_run = self.make_run(
                root,
                run_id="complete-target-candidate",
                target_sha=baseline_run.target_sha,
                mode="bench-full",
            )
            for run in (baseline_run, candidate_run):
                populate_benchmark_evidence(run, criterion_targets=targets)
                run.finalize()
            (root / "policy.json").write_text(
                baseline.policy_json_text(disabled_policy_fixture())
            )
            baseline_relative = baseline_run.run_dir.relative_to(root.resolve()).as_posix()
            candidate_relative = candidate_run.run_dir.relative_to(root.resolve()).as_posix()
            self.assertEqual(
                baseline.controlled_evaluate_artifacts(
                    root, "policy.json", baseline_relative, candidate_relative
                )["performance_enforcement"]["state"],
                "not_eligible",
            )

            def remove_target(run: baseline.ArtifactRun, target_root: str) -> None:
                shutil.rmtree(run.run_dir / "criterion" / "raw" / target_root)
                summary_path = run.run_dir / "summary.json"
                summary = json.loads(summary_path.read_text())
                marker = f"/criterion/raw/{target_root}/"
                retained = [
                    result
                    for result in summary["criterion_results"]
                    if marker not in f"/{result['source']}"
                ]
                self.assertEqual(
                    len(summary["criterion_results"]) - len(retained), 1
                )
                summary["criterion_results"] = retained
                baseline.write_json(summary_path, summary)
                baseline.write_json(
                    run.run_dir / "criterion" / "parsed-estimates.json", retained
                )
                baseline.write_checksums(root, run.run_dir)

            for run in (baseline_run, candidate_run):
                remove_target(run, "01-tcp_pool")

            with self.assertRaisesRegex(baseline.BaselineError, "target coverage"):
                baseline.controlled_evaluate_artifacts(
                    root, "policy.json", baseline_relative, candidate_relative
                )

            script = root / "scripts" / "baseline.py"
            script.parent.mkdir()
            shutil.copyfile(Path(baseline.__file__), script)
            completed = subprocess.run(
                (
                    sys.executable,
                    str(script),
                    "controlled-evaluate",
                    "policy.json",
                    baseline_relative,
                    candidate_relative,
                ),
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 1)
            self.assertEqual(completed.stdout, "")
            self.assertIn("Criterion target coverage", completed.stderr)

    def test_full_report_rejects_extra_criterion_target_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(
                root,
                run_id="extra-target",
                mode="bench-full",
                benchmark_targets=("tcp_throughput",),
            )
            populate_benchmark_evidence(run)
            run.finalize()

            extra_home = run.run_dir / "criterion" / "raw" / "02-tcp_extra"
            extra_estimate = extra_home / "tcp_extra" / "fixture" / "new" / "estimates.json"
            extra_estimate.parent.mkdir(parents=True)
            baseline.write_json(
                extra_estimate,
                {
                    "mean": {
                        "confidence_interval": {
                            "confidence_level": 0.95,
                            "lower_bound": 9.0,
                            "upper_bound": 11.0,
                        },
                        "point_estimate": 10.0,
                        "standard_error": 0.1,
                    }
                },
            )
            extra_results = baseline.parse_criterion_estimates(extra_home, root)
            summary_path = run.run_dir / "summary.json"
            summary = json.loads(summary_path.read_text())
            summary["criterion_results"].extend(extra_results)
            summary["criterion_results"].sort(
                key=lambda item: item["source"].encode("utf-8")
            )
            baseline.write_json(summary_path, summary)
            baseline.write_json(
                run.run_dir / "criterion" / "parsed-estimates.json",
                summary["criterion_results"],
            )
            baseline.write_checksums(root, run.run_dir)
            with self.assertRaisesRegex(baseline.BaselineError, "canonical registered target root"):
                baseline.build_benchmark_report(root, run.run_dir)

    def test_report_render_rejects_overwrite_traversal_and_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(root, run_id="safe-output")
            populate_benchmark_evidence(run)
            run.finalize()
            baseline.render_report_to_directory(root, run.run_dir, "reports/existing")
            with self.assertRaisesRegex(baseline.BaselineError, "already exists"):
                baseline.render_report_to_directory(root, run.run_dir, "reports/existing")
            with self.assertRaisesRegex(baseline.BaselineError, "path traversal"):
                baseline.render_report_to_directory(root, run.run_dir, "../outside")
            with self.assertRaisesRegex(baseline.BaselineError, "source artifact"):
                baseline.render_report_to_directory(
                    root, run.run_dir, str(run.run_dir / "rendered")
                )

            target = root / "symlink-target"
            target.mkdir()
            link = root / "report-link"
            try:
                link.symlink_to(target, target_is_directory=True)
            except OSError:
                self.skipTest("directory symlinks are unavailable")
            with self.assertRaisesRegex(baseline.BaselineError, "symlinks"):
                baseline.render_report_to_directory(root, run.run_dir, "report-link/new")

    def test_report_rejects_invalid_sources_and_unknown_versions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)

            checksum_root = parent / "checksum"
            checksum_root.mkdir()
            checksum_run = self.make_report_run(checksum_root, run_id="checksum")
            populate_benchmark_evidence(checksum_run)
            checksum_run.finalize()
            (checksum_run.run_dir / "summary.csv").write_text("tampered\n")
            with self.assertRaisesRegex(baseline.BaselineError, "checksum mismatch"):
                baseline.build_benchmark_report(checksum_root, checksum_run.run_dir)

            producer_root = parent / "producer"
            producer_root.mkdir()
            producer_run = self.make_report_run(producer_root, run_id="producer")
            populate_benchmark_evidence(producer_run)
            producer_run.finalize()
            provenance_path = producer_run.run_dir / "provenance.json"
            provenance = json.loads(provenance_path.read_text())
            provenance["harness_version"] = "unknown"
            baseline.write_json(provenance_path, provenance)
            baseline.write_checksums(producer_root, producer_run.run_dir)
            with self.assertRaisesRegex(baseline.BaselineError, "producer version"):
                baseline.build_benchmark_report(producer_root, producer_run.run_dir)

            missing_root = parent / "missing"
            missing_root.mkdir()
            missing_run = self.make_report_run(missing_root, run_id="missing")
            populate_benchmark_evidence(missing_run)
            missing_run.finalize()
            criterion = json.loads((missing_run.run_dir / "summary.json").read_text())[
                "criterion_results"
            ][0]
            (missing_root / criterion["source"]).unlink()
            baseline.write_checksums(missing_root, missing_run.run_dir)
            with self.assertRaisesRegex(baseline.BaselineError, "is missing"):
                baseline.build_benchmark_report(missing_root, missing_run.run_dir)

            partial_root = parent / "partial"
            partial_root.mkdir()
            partial_run = self.make_report_run(partial_root, run_id="partial")
            populate_benchmark_evidence(partial_run)
            partial_criterion = partial_run.criterion_results[0]
            (partial_root / partial_criterion["source"]).unlink()
            with self.assertRaisesRegex(baseline.BaselineError, "report generation failed"):
                partial_run.finalize()
            self.assertTrue((partial_run.run_dir / "checksums.sha256").is_file())
            partial_summary = json.loads((partial_run.run_dir / "summary.json").read_text())
            self.assertEqual(partial_summary["status"], "failed")
            self.assertFalse(partial_summary["baseline_valid"])

    def test_report_rejects_malformed_stress_and_criterion_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)

            stress_root = parent / "stress"
            stress_root.mkdir()
            stress_run = self.make_report_run(stress_root, run_id="bad-stress")
            populate_benchmark_evidence(stress_run)
            stress_run.finalize()
            summary_path = stress_run.run_dir / "summary.json"
            summary = json.loads(summary_path.read_text())
            sample = summary["stress_samples"][0]
            sample["errors"] = 1
            parsed_path = (
                stress_run.run_dir
                / "stress"
                / "parsed"
                / "stress-read-d1-r1.json"
            )
            baseline.write_json(parsed_path, sample)
            baseline.write_json(summary_path, summary)
            baseline.write_checksums(stress_root, stress_run.run_dir)
            with self.assertRaisesRegex(baseline.BaselineError, "errors must be zero"):
                baseline.build_benchmark_report(stress_root, stress_run.run_dir)

            criterion_root = parent / "criterion"
            criterion_root.mkdir()
            criterion_run = self.make_report_run(criterion_root, run_id="bad-criterion")
            populate_benchmark_evidence(criterion_run)
            criterion_run.finalize()
            summary_path = criterion_run.run_dir / "summary.json"
            summary = json.loads(summary_path.read_text())
            result = summary["criterion_results"][0]
            result["estimates"]["mean"]["point_estimate"] = "not-a-number"
            source_path = criterion_root / result["source"]
            baseline.write_json(source_path, result["estimates"])
            baseline.write_json(summary_path, summary)
            baseline.write_json(
                criterion_run.run_dir / "criterion" / "parsed-estimates.json",
                summary["criterion_results"],
            )
            baseline.write_checksums(criterion_root, criterion_run.run_dir)
            with self.assertRaisesRegex(baseline.BaselineError, "must be numeric"):
                baseline.build_benchmark_report(criterion_root, criterion_run.run_dir)

    def test_report_build_rejects_non_scalar_artifact_selectors(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(root, run_id="artifact-selectors")
            populate_benchmark_evidence(run)
            run.finalize()
            originals = {
                name: json.loads((run.run_dir / name).read_text())
                for name in ("environment.json", "provenance.json", "summary.json")
            }
            cases = [
                (
                    "power-availability",
                    "environment.json",
                    ("power", "availability"),
                    "runner.environment.power.availability must be a non-empty string",
                ),
                (
                    "provenance-mode",
                    "provenance.json",
                    ("mode",),
                    "provenance.json mode must be a non-empty string",
                ),
                (
                    "summary-mode",
                    "summary.json",
                    ("mode",),
                    "summary.json mode must be a non-empty string",
                ),
                (
                    "summary-status",
                    "summary.json",
                    ("status",),
                    "summary.json status must be a non-empty string",
                ),
            ]
            stress_selector_errors = {
                "transport": "stress.transport must be a non-empty string",
                "operation": "stress.operation must be a non-empty string",
                "in_flight": "stress.in_flight must be an integer >= 1",
                "clients": "stress.clients must be an integer >= 1",
                "registers": "stress.registers must be an integer >= 1",
                "repetition": "stress.repetition must be an integer >= 1",
                "duration_secs": "stress.duration_secs must be an integer >= 1",
                "warmup_secs": "stress.warmup_secs must be an integer >= 0",
                "command_id": "stress.command_id is missing or malformed",
            }
            for field, expected_error in stress_selector_errors.items():
                cases.append(
                    (
                        f"stress-{field}",
                        "summary.json",
                        ("stress_samples", 0, field),
                        expected_error,
                    )
                )

            for label, document_name, path, expected_error in cases:
                for value_label, invalid_value in (
                    ("list", [label]),
                    ("object", {"selector": label}),
                ):
                    with self.subTest(selector=label, value=value_label):
                        documents = copy.deepcopy(originals)
                        target = documents[document_name]
                        for part in path[:-1]:
                            target = target[part]
                        target[path[-1]] = invalid_value
                        for name, document in documents.items():
                            baseline.write_json(run.run_dir / name, document)
                        baseline.write_checksums(root, run.run_dir)
                        with self.assertRaises(baseline.BaselineError) as raised:
                            baseline.build_benchmark_report(root, run.run_dir)
                        self.assertIn(expected_error, str(raised.exception))

    def test_report_document_rejects_unknown_schema_and_producer(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_report_run(root, run_id="report-validation")
            populate_benchmark_evidence(run)
            run.finalize()
            report = baseline.build_benchmark_report(root, run.run_dir)

            unknown_schema = copy.deepcopy(report)
            unknown_schema["report_schema"]["version"] = 2
            self.assertTrue(baseline.validate_report_document(unknown_schema))
            unknown_producer = copy.deepcopy(report)
            unknown_producer["producers"][-1]["version"] = "unknown"
            self.assertTrue(baseline.validate_report_document(unknown_producer))

    def test_report_rejects_non_scalar_selector_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            run = self.make_report_run(root, run_id="invalid-report-selectors")
            populate_benchmark_evidence(run)
            run.finalize()
            report = baseline.build_benchmark_report(root, run.run_dir)
            stress_index = next(
                index
                for index, scenario in enumerate(report["scenarios"])
                if scenario["kind"] == "tcp_stress"
            )
            cases = [
                (
                    "run-mode",
                    ("run", "mode"),
                    "run.mode must be a non-empty string",
                ),
                (
                    "power-availability",
                    ("runner", "environment", "power", "availability"),
                    "runner.environment.power.availability must be a non-empty string",
                ),
                (
                    "scenario-kind",
                    ("scenarios", stress_index, "kind"),
                    f"scenario {stress_index}.kind must be a non-empty string",
                ),
            ]
            stress_identity_selector_fields = (
                "clients",
                "duration_seconds",
                "in_flight",
                "operation",
                "registers",
                "repetitions",
                "transport",
                "warmup_seconds",
            )
            for field in stress_identity_selector_fields:
                scalar_contract = (
                    "a non-empty string"
                    if field in {"operation", "transport"}
                    else "an integer"
                )
                minimum = " >= 1" if field in {
                    "clients",
                    "in_flight",
                    "registers",
                    "repetitions",
                } else " >= 0" if field in {"duration_seconds", "warmup_seconds"} else ""
                cases.append(
                    (
                        f"stress-identity-{field}",
                        ("scenarios", stress_index, "identity", field),
                        f"scenario {stress_index}: identity.{field} must be "
                        f"{scalar_contract}{minimum}",
                    )
                )

            for selector, path, expected_error in cases:
                for value_label, invalid_value in (
                    ("list", [selector]),
                    ("object", {"selector": selector}),
                ):
                    with self.subTest(selector=selector, value=value_label):
                        malformed = copy.deepcopy(report)
                        target = malformed
                        for part in path[:-1]:
                            target = target[part]
                        target[path[-1]] = invalid_value
                        self.assertIn(
                            expected_error,
                            baseline.validate_report_document(malformed),
                        )

                        report_path = root / f"malformed-{selector}-{value_label}.json"
                        baseline.write_json(report_path, malformed)
                        with self.assertRaises(baseline.BaselineError) as raised:
                            baseline.load_report_file(root, str(report_path))
                        self.assertIn(expected_error, str(raised.exception))

                        with self.assertRaises(baseline.BaselineError) as raised:
                            baseline.report_json_text(malformed)
                        self.assertIn(expected_error, str(raised.exception))

                        with self.assertRaises(baseline.BaselineError) as raised:
                            baseline.render_report_markdown(malformed)
                        self.assertIn(expected_error, str(raised.exception))

                        stderr = io.StringIO()
                        with mock.patch.object(
                            baseline, "__file__", str(root / "scripts" / "baseline.py")
                        ), contextlib.redirect_stdout(
                            io.StringIO()
                        ), contextlib.redirect_stderr(stderr):
                            self.assertEqual(
                                baseline.main(["validate-report", str(report_path)]), 1
                            )
                        self.assertIn(expected_error, stderr.getvalue())

    def test_cli_report_validation_rejects_malformed_render_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            run = self.make_report_run(root, run_id="cli-validation")
            populate_benchmark_evidence(run)
            run.finalize()
            report = baseline.build_benchmark_report(root, run.run_dir)
            stress = next(item for item in report["scenarios"] if item["kind"] == "tcp_stress")
            criterion = next(
                item for item in report["scenarios"] if item["kind"] == "criterion_estimate"
            )
            cases = {
                "missing-stress-mean": lambda value: value["scenarios"][
                    report["scenarios"].index(stress)
                ]["metrics"]["throughput"]["recorded_statistics"].pop("mean"),
                "boolean-stress-mean": lambda value: value["scenarios"][
                    report["scenarios"].index(stress)
                ]["metrics"]["throughput"]["recorded_statistics"].update({"mean": True}),
                "nonfinite-stress-max": lambda value: value["scenarios"][
                    report["scenarios"].index(stress)
                ]["metrics"]["p99_latency"]["recorded_statistics"].update(
                    {"max": math.inf}
                ),
                "overflow-stress-mean": lambda value: value["scenarios"][
                    report["scenarios"].index(stress)
                ]["metrics"]["throughput"]["recorded_statistics"].update(
                    {"mean": 10**1000}
                ),
                "missing-criterion-lower": lambda value: value["scenarios"][
                    report["scenarios"].index(criterion)
                ]["metrics"]["mean_estimate"].pop("lower"),
                "string-criterion-upper": lambda value: value["scenarios"][
                    report["scenarios"].index(criterion)
                ]["metrics"]["mean_estimate"].update({"upper": "11.0"}),
                "incoherent-criterion-bounds": lambda value: value["scenarios"][
                    report["scenarios"].index(criterion)
                ]["metrics"]["mean_estimate"].update({"lower": 12.0}),
                "missing-confidence-level": lambda value: value["scenarios"][
                    report["scenarios"].index(criterion)
                ]["metrics"]["mean_estimate"].pop("confidence_level"),
                "boolean-standard-error": lambda value: value["scenarios"][
                    report["scenarios"].index(criterion)
                ]["metrics"]["mean_estimate"].update({"standard_error": True}),
                "missing-checksum-reference": lambda value: value["source_artifact"].pop(
                    "checksum_inventory"
                ),
                "missing-source-timestamp": lambda value: value["source_artifact"][
                    "provenance"
                ].pop("ended_utc"),
            }
            for index, (label, mutate) in enumerate(cases.items()):
                with self.subTest(case=label):
                    malformed = copy.deepcopy(report)
                    mutate(malformed)
                    report_path = root / f"malformed-{index}.json"
                    baseline.write_json(report_path, malformed)
                    with self.assertRaises(baseline.BaselineError):
                        baseline.load_report_file(root, str(report_path))
                    with self.assertRaises(baseline.BaselineError):
                        baseline.report_json_text(malformed)
                    with self.assertRaises(baseline.BaselineError):
                        baseline.render_report_markdown(malformed)
                    with mock.patch.object(
                        baseline, "__file__", str(root / "scripts" / "baseline.py")
                    ), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                        io.StringIO()
                    ):
                        self.assertEqual(
                            baseline.main(["validate-report", str(report_path)]), 1
                        )

            valid_path = root / "valid-report.json"
            baseline.write_json(valid_path, report)
            loaded = baseline.load_report_file(root, str(valid_path))
            self.assertEqual(loaded, report)
            self.assertEqual(
                baseline.render_report_markdown(loaded),
                baseline.render_report_markdown(report),
            )
            with mock.patch.object(
                baseline, "__file__", str(root / "scripts" / "baseline.py")
            ), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                io.StringIO()
            ):
                self.assertEqual(baseline.main(["validate-report", str(valid_path)]), 0)
                self.assertEqual(
                    baseline.main(
                        [
                            "report",
                            run.run_dir.relative_to(root).as_posix(),
                            "--output-dir",
                            "cli-render",
                        ]
                    ),
                    0,
                )
                self.assertEqual(
                    baseline.main(
                        ["validate-report", "cli-render/benchmark-report-v1.json"]
                    ),
                    0,
                )

    def test_criterion_identity_is_proven_from_target_sha_lock(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory).resolve()
            valid_root = parent / "valid"
            valid_root.mkdir()
            run = self.make_report_run(valid_root, run_id="lock-proof")
            populate_benchmark_evidence(run)
            run.finalize()
            report = baseline.build_benchmark_report(valid_root, run.run_dir)
            criterion_producer = next(
                item
                for item in report["producers"]
                if item["id"] == baseline.CRITERION_PRODUCER_ID
            )
            self.assertEqual(criterion_producer["version"], "0.5.1")
            self.assertEqual(
                baseline.criterion_version_from_target_lock(
                    valid_root, report["run"]["target_sha"]
                ),
                "0.5.1",
            )

            (valid_root / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "criterion"\nversion = "0.5.2"\n'
            )
            subprocess.run(
                ("git", "add", "Cargo.lock"),
                cwd=valid_root,
                check=True,
                capture_output=True,
            )
            subprocess.run(
                (
                    "git",
                    "-c",
                    "user.name=Benchmark Report Test",
                    "-c",
                    "user.email=benchmark-report@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "unsupported lock",
                ),
                cwd=valid_root,
                check=True,
                capture_output=True,
            )
            unsupported_sha = subprocess.run(
                ("git", "rev-parse", "HEAD"),
                cwd=valid_root,
                check=True,
                capture_output=True,
            ).stdout.decode().strip()
            mismatch = copy.deepcopy(report)
            mismatch["run"]["target_sha"] = unsupported_sha
            source_parts = mismatch["source_artifact"]["path"].split("/")
            source_parts[-2] = unsupported_sha
            mismatch["source_artifact"]["path"] = "/".join(source_parts)
            mismatch_path = valid_root / "mismatch-report.json"
            baseline.write_json(mismatch_path, mismatch)
            with self.assertRaisesRegex(baseline.BaselineError, "does not match"):
                baseline.load_report_file(valid_root, str(mismatch_path))

            unsupported_root = parent / "unsupported"
            unsupported_root.mkdir()
            unsupported_run = self.make_report_run(
                unsupported_root,
                run_id="unsupported",
                criterion_versions=("0.5.2",),
            )
            populate_benchmark_evidence(unsupported_run)
            with self.assertRaisesRegex(baseline.BaselineError, "unsupported"):
                unsupported_run.finalize()

            ambiguous_root = parent / "ambiguous"
            ambiguous_root.mkdir()
            ambiguous_sha = initialize_git_lock_fixture(
                ambiguous_root, criterion_versions=("0.5.1", "0.5.1")
            )
            with self.assertRaisesRegex(baseline.BaselineError, "ambiguous"):
                baseline.verified_criterion_version(ambiguous_root, ambiguous_sha)

            unlabelled_root = parent / "unlabelled"
            unlabelled_root.mkdir()
            unlabelled_sha = initialize_git_lock_fixture(
                unlabelled_root, criterion_versions=()
            )
            with self.assertRaisesRegex(baseline.BaselineError, "does not identify"):
                baseline.verified_criterion_version(unlabelled_root, unlabelled_sha)

            missing_root = parent / "missing-lock"
            missing_root.mkdir()
            missing_sha = initialize_git_lock_fixture(missing_root, include_lock=False)
            with self.assertRaisesRegex(baseline.BaselineError, "unavailable"):
                baseline.verified_criterion_version(missing_root, missing_sha)

            with self.assertRaisesRegex(baseline.BaselineError, "unavailable"):
                baseline.verified_criterion_version(valid_root, "b" * 40)

    def test_checksum_inventory_verifies_and_detects_tampering(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_run(root)
            run.finalize()
            self.assertEqual(baseline.validate_artifact(root, run.run_dir), [])
            original_manifest = (run.run_dir / "checksums.sha256").read_bytes()
            baseline.write_checksums(root, run.run_dir)
            self.assertEqual((run.run_dir / "checksums.sha256").read_bytes(), original_manifest)
            manifest_lines = (run.run_dir / "checksums.sha256").read_text().splitlines()
            paths = [line.split("  ", 1)[1] for line in manifest_lines]
            self.assertEqual(paths, sorted(paths, key=lambda item: item.encode("utf-8")))
            (run.run_dir / "summary.csv").write_text("tampered\n")
            self.assertTrue(
                any("checksum mismatch" in error for error in baseline.validate_artifact(root, run.run_dir))
            )

    def test_checksum_validator_rejects_nondeterministic_order(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_run(root)
            run.finalize()
            manifest = run.run_dir / "checksums.sha256"
            lines = manifest.read_text().splitlines()
            manifest.write_text("\n".join(reversed(lines)) + "\n")
            self.assertIn(
                "checksum paths are not in bytewise order",
                baseline.validate_artifact(root, run.run_dir),
            )

    def test_validator_rejects_coherence_mismatches_after_checksums_are_regenerated(
        self,
    ) -> None:
        cases = (
            ("summary.json", "mode", "bench-full", "mode do not match"),
            ("provenance.json", "run_id", "other-run", "run_id does not match artifact path"),
            ("summary.json", "target_sha", "b" * 40, "target_sha does not match artifact path"),
            ("summary.json", "status", "failed", "status does not agree with baseline_valid"),
        )
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            for index, (name, field, value, expected_error) in enumerate(cases):
                with self.subTest(field=field):
                    root = parent / str(index)
                    root.mkdir()
                    run = self.make_run(root, run_id=f"coherence-{index}")
                    run.finalize()
                    path = run.run_dir / name
                    document = json.loads(path.read_text())
                    document[field] = value
                    baseline.write_json(path, document)
                    baseline.write_checksums(root, run.run_dir)

                    errors = baseline.validate_artifact(root, run.run_dir)
                    self.assertFalse(any("checksum mismatch" in error for error in errors))
                    self.assertTrue(
                        any(expected_error in error for error in errors),
                        errors,
                    )

    def test_subprocess_failure_finalizes_partial_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_run(root)
            spec = baseline.CommandSpec(
                "fixture-failure",
                (
                    sys.executable,
                    "-c",
                    "import sys; print('out'); print('err', file=sys.stderr); raise SystemExit(7)",
                ),
                root,
            )
            with self.assertRaises(baseline.CommandFailure) as caught:
                run.run_command(spec)
            run.add_error(caught.exception)
            run.finalize()
            command_dir = next((run.run_dir / "commands").iterdir())
            self.assertEqual((command_dir / "command.stdout").read_text(), "out\n")
            self.assertEqual((command_dir / "command.stderr").read_text(), "err\n")
            self.assertTrue((run.run_dir / "checksums.sha256").is_file())
            summary = json.loads((run.run_dir / "summary.json").read_text())
            self.assertEqual(summary["status"], "failed")

    def test_missing_command_is_recorded_and_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_run(root)
            with self.assertRaises(baseline.CommandFailure):
                run.run_command(
                    baseline.CommandSpec(
                        "missing-tool", ("definitely-not-a-real-command-7f9d",), root
                    )
                )
            record = json.loads(
                next((run.run_dir / "commands").glob("*/command.json")).read_text()
            )
            self.assertIsNone(record["exit_code"])
            self.assertEqual(record["argv"], ["definitely-not-a-real-command-7f9d"])

    def test_command_records_exact_argv_cwd_and_raw_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = self.make_run(root)
            argv = (sys.executable, "-c", "print('fixture')")
            result = run.run_command(baseline.CommandSpec("fixture", argv, root))
            record_path = run.run_dir / "commands" / result.command_id / "command.json"
            record = json.loads(record_path.read_text())
            self.assertEqual(record["argv"], list(argv))
            self.assertEqual(record["cwd"], str(root.resolve()))
            self.assertEqual(
                (record_path.parent / "command.stdout").read_bytes(), b"fixture\n"
            )

    def test_stress_json_contract_and_scenario_are_strict(self) -> None:
        payload = stress_fixture()
        parsed = baseline.parse_stress_json(json.dumps(payload), expected_scenario())
        self.assertEqual(parsed["schema_version"], 1)
        for field in ("throughput_ops_sec", "registers", "warmup_secs"):
            invalid = copy.deepcopy(payload)
            invalid.pop(field)
            with self.subTest(field=field), self.assertRaises(baseline.BaselineError):
                baseline.parse_stress_json(json.dumps(invalid), expected_scenario())
        with self.assertRaises(baseline.BaselineError):
            baseline.parse_stress_json(json.dumps(payload), expected_scenario(in_flight=1))
        with self.assertRaises(baseline.BaselineError):
            baseline.parse_stress_json(b"not-json", expected_scenario())

    def test_stress_errors_error_rate_and_retries_fail_closed(self) -> None:
        for field, value in (("errors", 1), ("error_rate", 0.01), ("retry_attempts", 1)):
            with self.subTest(field=field), self.assertRaises(baseline.BaselineError):
                baseline.parse_stress_json(
                    json.dumps(stress_fixture(**{field: value})), expected_scenario()
                )

    def test_stress_numeric_types_and_finiteness_are_strict(self) -> None:
        for field, value in (
            ("total_ops", True),
            ("throughput_ops_sec", "100"),
            ("throughput_ops_sec", math.inf),
            ("error_rate", -1.0),
        ):
            with self.subTest(field=field), self.assertRaises(baseline.BaselineError):
                baseline.parse_stress_json(
                    json.dumps(stress_fixture(**{field: value})), expected_scenario()
                )

    def test_stress_hash_selector_fields_reject_non_scalar_values(self) -> None:
        expected = expected_scenario()
        expected["repetition"] = 1
        sample = stress_fixture()
        sample["repetition"] = 1
        selector_fields = (
            "transport",
            "operation",
            "in_flight",
            "clients",
            "registers",
            "repetition",
        )
        for field in selector_fields:
            for value_label, invalid_value in (
                ("list", [field]),
                ("object", {"selector": field}),
            ):
                with self.subTest(selector=field, value=value_label):
                    malformed = copy.deepcopy(sample)
                    malformed[field] = invalid_value
                    with self.assertRaises(baseline.BaselineError):
                        baseline.aggregate_stress_samples([malformed], [expected])

    def test_five_repetition_completeness_and_duplicate_detection(self) -> None:
        expected = baseline.stress_scenarios("bench-full", 5)
        samples = []
        for scenario in expected:
            sample = stress_fixture(
                operation=scenario["operation"],
                in_flight=scenario["in_flight"],
            )
            sample["repetition"] = scenario["repetition"]
            samples.append(sample)
        aggregates = baseline.aggregate_stress_samples(samples, expected)
        self.assertEqual(len(aggregates), 10)
        self.assertTrue(all(item["repetitions"] == 5 for item in aggregates))
        with self.assertRaises(baseline.BaselineError):
            baseline.aggregate_stress_samples(samples[:-1], expected)
        with self.assertRaises(baseline.BaselineError):
            baseline.aggregate_stress_samples(samples + [copy.deepcopy(samples[0])], expected)

    def test_aggregate_math_and_zero_mean_cv(self) -> None:
        stats = baseline.sample_statistics([1.0, 2.0, 3.0])
        self.assertEqual(stats["median"], 2.0)
        self.assertEqual(stats["mean"], 2.0)
        self.assertEqual(stats["sample_stddev"], 1.0)
        self.assertEqual(stats["coefficient_of_variation"], 0.5)
        self.assertIsNone(baseline.sample_statistics([0.0, 0.0])["coefficient_of_variation"])

    def test_criterion_estimates_are_discovered_and_parsed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            criterion = root / "artifact/criterion"
            estimate = criterion / "group/case/new/estimates.json"
            estimate.parent.mkdir(parents=True)
            baseline.write_json(
                estimate,
                {
                    "mean": {
                        "confidence_interval": {
                            "confidence_level": 0.95,
                            "lower_bound": 9.0,
                            "upper_bound": 11.0,
                        },
                        "point_estimate": 10.0,
                        "standard_error": 0.1,
                    }
                },
            )
            parsed = baseline.parse_criterion_estimates(criterion, root)
            self.assertEqual(parsed[0]["benchmark_id"], "group/case")
            self.assertEqual(parsed[0]["mean_ns"]["point"], 10.0)
            with self.assertRaises(baseline.BaselineError):
                baseline.parse_criterion_estimates(root / "missing", root)

    def test_criterion_missing_or_malformed_estimates_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            estimate = root / "criterion/case/new/estimates.json"
            estimate.parent.mkdir(parents=True)
            baseline.write_json(estimate, {"median": {}})
            with self.assertRaises(baseline.BaselineError):
                baseline.parse_criterion_estimates(root / "criterion", root)

    def test_runner_environment_is_allowlisted(self) -> None:
        environ = {
            "GITHUB_RUN_ID": "123",
            "RUNNER_OS": "Linux",
            "SECRET_TOKEN": "must-not-leak",
        }
        self.assertEqual(
            baseline.allowlisted_runner_environment(environ),
            {"GITHUB_RUN_ID": "123", "RUNNER_OS": "Linux"},
        )

    def test_absent_cpu_and_power_metadata_are_portable(self) -> None:
        with mock.patch.object(baseline.platform, "processor", return_value=""), mock.patch.object(
            baseline.Path, "read_text", side_effect=OSError("missing")
        ):
            cpu = baseline.cpu_metadata()
            power = baseline.power_metadata()
        self.assertIn("logical_count", cpu)
        self.assertEqual(power["availability"], "unavailable")
        self.assertIsNone(power["value"])

    def test_cargo_metadata_summary_discovers_registered_benches(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = {
                "workspace_root": str(root),
                "target_directory": str(root / "target"),
                "workspace_members": ["bench"],
                "packages": [
                    {
                        "name": "rusty-modbus-benchmarks",
                        "version": "0.1.0",
                        "targets": [
                            {"name": "codec", "kind": ["bench"]},
                            {"name": "stress-test", "kind": ["bin"]},
                        ],
                    }
                ],
            }
            summary, benches, target = baseline.summarize_cargo_metadata(
                json.dumps(payload).encode()
            )
        self.assertEqual(benches, ["codec"])
        self.assertEqual(target, root / "target")
        self.assertEqual(summary["workspace_member_count"], 1)

    def test_mode_plans_match_required_defaults_without_thresholds(self) -> None:
        correctness = baseline.correctness_plan(ROOT)
        argvs = [spec.argv for spec in correctness]
        self.assertIn(
            ("cargo", "nextest", "run", "--workspace", "--locked", "--profile", "ci"),
            argvs,
        )
        self.assertIn(
            (
                "cargo",
                "nextest",
                "run",
                "-p",
                "rusty-modbus-conformance",
                "--locked",
                "--profile",
                "ci",
            ),
            argvs,
        )
        self.assertIn(("cargo", "audit", "--ignore", "RUSTSEC-2025-0134"), argvs)
        self.assertFalse(any("threshold" in part for argv in argvs for part in argv))

        smoke = baseline.stress_scenarios("bench-smoke", 1)
        full = baseline.stress_scenarios("bench-full", 5)
        self.assertEqual(len(smoke), 6)
        self.assertEqual({item["in_flight"] for item in smoke}, {1, 8, 16})
        self.assertEqual(len(full), 50)
        self.assertEqual({item["in_flight"] for item in full}, {1, 2, 4, 8, 16})

        specs = baseline.benchmark_criterion_specs(
            "bench-smoke",
            ROOT,
            ["codec", "tcp_pool", "tcp_throughput"],
            ROOT / "artifact",
        )
        self.assertEqual(
            [spec.label for spec in specs],
            ["criterion-tcp_throughput", "criterion-tcp_pool"],
        )
        self.assertTrue(all(spec.argv[-2:] == ("--quick", "--noplot") for spec in specs))
        self.assertIn("tcp_pipelined", specs[0].argv)
        self.assertEqual(
            specs[1].argv,
            (
                "cargo",
                "bench",
                "-p",
                "rusty-modbus-benchmarks",
                "--bench",
                "tcp_pool",
                "--locked",
                "--",
                "--quick",
                "--noplot",
            ),
        )

        full_specs = baseline.benchmark_criterion_specs(
            "bench-full",
            ROOT,
            [
                "codec",
                "rtu_tcp_latency",
                "server_handler",
                "tcp_latency",
                "tcp_pool",
                "tcp_throughput",
                "tls_latency",
            ],
            ROOT / "artifact",
        )
        self.assertEqual(
            [spec.label for spec in full_specs],
            [
                "criterion-tcp_latency",
                "criterion-tcp_pool",
                "criterion-tcp_throughput",
            ],
        )

    def test_cli_mode_defaults_are_bounded(self) -> None:
        parser = baseline.build_parser()
        smoke = parser.parse_args(["bench-smoke", "--runner-label", "local"])
        self.assertEqual((smoke.duration, smoke.warmup, smoke.repetitions), (1, 1, 1))
        full = parser.parse_args(["bench-full", "--runner-label", "local"])
        self.assertEqual((full.duration, full.warmup, full.repetitions), (5, 1, 5))
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(["bench-smoke", "--runner-label", "local", "--duration", "0"])


class ArtifactFingerprintTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve() / "repository"
        self.root.mkdir()
        self.targets = ("tcp_pool", "tcp_throughput")
        self.sha = initialize_git_lock_fixture(self.root, benchmark_targets=self.targets)
        scripts = self.root / "scripts"
        scripts.mkdir()
        self.script = scripts / "baseline.py"
        shutil.copyfile(SCRIPTS / "baseline.py", self.script)
        self.artifact = self.make_artifact()
        self.relative = self.artifact.run_dir.relative_to(self.root).as_posix()

    def make_artifact(
        self, *, run_id: str = "fingerprint-smoke", mode: str = "bench-smoke",
        dirty: bool = False, failed: bool = False,
    ) -> baseline.ArtifactRun:
        run = baseline.ArtifactRun(
            repo_root=self.root, output_root=self.root / "retained espace-é",
            target_sha=self.sha, run_id=run_id, mode=mode,
            runner_label="synthetic-fingerprint-runner", dirty=dirty, allow_dirty=dirty,
        )
        run.create()
        if mode in baseline.BENCHMARK_MODES:
            populate_benchmark_evidence(
                run, criterion_targets=self.targets if mode == "bench-full" else ("tcp_throughput",)
            )
        else:
            run.environment = environment_fixture()
        if failed:
            run.add_error("synthetic failure")
        run.finalize()
        return run

    def cli(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.script), *args], cwd=self.root.parent,
            capture_output=True, text=True, encoding="utf-8", timeout=15,
        )

    def inventory(self, root: Path | None = None) -> dict:
        root = root or self.root
        return {
            path.relative_to(root).as_posix(): (
                ("symlink", str(path.readlink())) if path.is_symlink()
                else ("file", hashlib.sha256(path.read_bytes()).hexdigest()) if path.is_file()
                else ("directory", None) if path.is_dir()
                else ("special", None)
            )
            for path in root.rglob("*")
        }

    def test_content_preimage_has_an_independent_known_vector(self) -> None:
        # SHA256("") and SHA256("abc"); preimage digest cross-checked with openssl.
        empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        files = [{"sha256": abc, "path": "é.txt"}, {"sha256": empty, "path": "a.txt"}]
        original = copy.deepcopy(files)
        content, digest = baseline._artifact_content_fingerprint(files)
        expected = (
            '{"content_schema":{"name":"benchmark-artifact-content","version":1},'
            '"files":[{"path":"a.txt","sha256":"' + empty
            + '"},{"path":"é.txt","sha256":"' + abc + '"}]}\n'
        )
        self.assertEqual(baseline.artifact_fingerprint_json_text(content), expected)
        self.assertEqual(digest, "d4e09959da94e5d1a8768ad6e8e7c6480c99b74803d8acd518a2a72193825562")
        self.assertEqual(files, original)

    def test_smoke_fingerprint_covers_actual_files_not_root_checksums(self) -> None:
        before = self.inventory()
        result = baseline.fingerprint_artifact(self.root, self.relative)
        expected_files = [
            {"path": path.relative_to(self.artifact.run_dir).as_posix(),
             "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for path in self.artifact.run_dir.rglob("*")
            if path.is_file() and path != self.artifact.run_dir / "checksums.sha256"
        ]
        expected_files.sort(key=lambda item: item["path"].encode("utf-8"))
        self.assertEqual(result["content"]["files"], expected_files)
        self.assertEqual(result["source"], {
            "target_sha": self.sha, "run_id": self.artifact.run_id, "mode": "bench-smoke",
        })
        self.assertEqual(self.inventory(), before)

    def test_cli_is_canonical_json_only_repeated_and_cwd_independent(self) -> None:
        before = self.inventory()
        first = self.cli("fingerprint-artifact", self.relative)
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(first.stderr, "")
        result = json.loads(first.stdout)
        self.assertEqual(result["fingerprint_schema"], {
            "name": "benchmark-artifact-fingerprint", "version": 1,
        })
        preimage = json.dumps(
            result["content"], sort_keys=True, ensure_ascii=False, separators=(",", ":"),
            allow_nan=False,
        ) + "\n"
        self.assertEqual(result["sha256"], hashlib.sha256(preimage.encode("utf-8")).hexdigest())
        self.assertEqual(first.stdout, json.dumps(
            result, sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False,
        ) + "\n")
        self.assertEqual(self.cli("fingerprint-artifact", self.relative).stdout, first.stdout)
        self.assertEqual(self.inventory(), before)

    def assert_fingerprint_failure(self, relative: str | None = None) -> None:
        relative = self.relative if relative is None else relative
        before = self.inventory()
        with self.assertRaises(baseline.BaselineError):
            baseline.fingerprint_artifact(self.root, relative)
        result = self.cli("fingerprint-artifact", relative)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertTrue(result.stderr.startswith("baseline: "), result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(self.inventory(), before)

    def test_content_names_raw_evidence_and_stored_reports_change_identity(self) -> None:
        run_dir = self.artifact.run_dir
        original = baseline.fingerprint_artifact(self.root, self.relative)["sha256"]
        for path in (
            next(run_dir.glob("commands/*/command.stdout")),
            run_dir / "benchmark-report-v1.json", run_dir / "benchmark-report-v1.md",
        ):
            with self.subTest(file=path.name):
                raw = path.read_bytes()
                path.write_bytes(raw + b"\n")
                baseline.write_checksums(self.root, run_dir)
                self.assertNotEqual(baseline.fingerprint_artifact(self.root, self.relative)["sha256"], original)
                path.write_bytes(raw)
        baseline.write_checksums(self.root, run_dir)
        self.assertEqual(baseline.fingerprint_artifact(self.root, self.relative)["sha256"], original)

        added = run_dir / "extra log-é.bin"
        added.write_bytes(b"same raw bytes\x00\xff")
        baseline.write_checksums(self.root, run_dir)
        after_add = baseline.fingerprint_artifact(self.root, self.relative)["sha256"]
        self.assertNotEqual(after_add, original)
        renamed = added.rename(run_dir / "other log-é.bin")
        baseline.write_checksums(self.root, run_dir)
        after_rename = baseline.fingerprint_artifact(self.root, self.relative)["sha256"]
        self.assertNotEqual(after_rename, after_add)
        renamed.unlink()
        baseline.write_checksums(self.root, run_dir)
        self.assertEqual(baseline.fingerprint_artifact(self.root, self.relative)["sha256"], original)
        (run_dir / "empty directory").mkdir()
        self.assertEqual(baseline.fingerprint_artifact(self.root, self.relative)["sha256"], original)

    def test_relocation_and_inventory_order_preserve_byte_identical_payload(self) -> None:
        result = baseline.fingerprint_artifact(self.root, self.relative)
        moved = self.root.parent / "different checkout-é"
        shutil.copytree(self.root, moved)
        before = self.inventory(moved)
        self.assertEqual(baseline.fingerprint_artifact(moved, self.relative), result)
        self.assertEqual(self.inventory(moved), before)
        content, digest = baseline._artifact_content_fingerprint(list(reversed(result["content"]["files"])))
        self.assertEqual(content, result["content"])
        self.assertEqual(digest, result["sha256"])
        # Unlike the inventory path, embedded absolute metadata bytes are not rewritten.
        command = next((moved / self.relative).glob("commands/*/command.json"))
        command.write_text(command.read_text().replace(str(self.root), str(moved)), encoding="utf-8")
        baseline.write_checksums(moved, moved / self.relative)
        self.assertNotEqual(baseline.fingerprint_artifact(moved, self.relative)["sha256"], digest)

    def test_root_checksums_are_verified_but_not_hashed(self) -> None:
        before = baseline.fingerprint_artifact(self.root, self.relative)
        manifest = self.artifact.run_dir / "checksums.sha256"
        raw = manifest.read_bytes()
        manifest.write_bytes(raw.replace(b"\n", b"\r\n"))
        self.assertEqual(baseline.fingerprint_artifact(self.root, self.relative), before)
        manifest.write_bytes(raw.rstrip(b"\n"))
        self.assertEqual(baseline.fingerprint_artifact(self.root, self.relative), before)
        manifest.write_bytes(b"0" * 64 + raw[64:])
        self.assert_fingerprint_failure()

    def test_unsafe_checksum_paths_and_inventories_fail_before_legacy_reads(self) -> None:
        manifest = self.artifact.run_dir / "checksums.sha256"
        raw = manifest.read_bytes()
        lines = raw.decode("utf-8").splitlines()
        digest, path = lines[0].split("  ", 1)
        aliases = (
            str(self.root / path), "../" + path, "./" + path,
            path.replace("/", "//", 1), path.replace("/", "/./", 1),
            path.replace("/", "/../", 1), path.replace("/", "\\", 1),
            "C:/outside.json", "outside.json", self.relative + "/checksums.sha256",
            self.relative + "/absent.json",
        )
        cases = [b"\xff", b"not a checksum\n", b"\n", b"", raw + (lines[0] + "\n").encode()]
        cases += [("\n".join(reversed(lines)) + "\n").encode(), ("\n".join(lines[1:]) + "\n").encode()]
        cases += [(digest + "  " + alias + "\n" + "\n".join(lines[1:]) + "\n").encode() for alias in aliases]
        for position, invalid in enumerate(cases):
            with self.subTest(case=position):
                manifest.write_bytes(invalid)
                with mock.patch.object(
                    baseline, "build_benchmark_report", side_effect=AssertionError("unsafe preflight")
                ), mock.patch.object(baseline, "_sha256", side_effect=AssertionError("unsafe hash")):
                    with self.assertRaises(baseline.BaselineError):
                        baseline.fingerprint_artifact(self.root, self.relative)
                self.assert_fingerprint_failure()

    def test_missing_unlisted_and_nested_checksum_files_are_not_ignored(self) -> None:
        run_dir = self.artifact.run_dir
        extra = run_dir / "unlisted.bin"
        extra.write_bytes(b"not listed")
        self.assert_fingerprint_failure()
        extra.unlink()
        missing = run_dir / "summary.csv"
        raw = missing.read_bytes()
        missing.unlink()
        self.assert_fingerprint_failure()
        missing.write_bytes(raw)
        nested = run_dir / "commands" / "checksums.sha256"
        nested.write_bytes(b"otherwise silently omitted by the legacy writer")
        baseline.write_checksums(self.root, run_dir)
        with mock.patch.object(baseline, "build_benchmark_report", side_effect=AssertionError("nested")):
            with self.assertRaisesRegex(baseline.BaselineError, "nested"):
                baseline.fingerprint_artifact(self.root, self.relative)
        self.assert_fingerprint_failure()

    def test_strict_directory_input_and_symlinks_are_rejected(self) -> None:
        for relative in ("", ".", "..", str(self.artifact.run_dir), "C:/artifact", "missing",
                         "./" + self.relative, self.relative + "/", self.relative.replace("/", "//", 1),
                         self.relative.replace("/", "\\", 1), self.relative + "/summary.json"):
            with self.subTest(path=relative):
                self.assert_fingerprint_failure(relative)
        with self.assertRaises(baseline.BaselineError):
            baseline.fingerprint_artifact(self.root, "bad\x00path")
        links = [
            (self.root / "artifact-link", self.artifact.run_dir, "artifact-link"),
            (self.root / "ancestor-link", self.artifact.run_dir.parent,
             "ancestor-link/" + self.artifact.run_id),
            (self.artifact.run_dir / "linked-directory", self.root / "scripts", self.relative),
            (self.artifact.run_dir / "dangling", self.root / "absent", self.relative),
            (self.artifact.run_dir / "linked-file", self.script, self.relative),
        ]
        for link, target, relative in links:
            with self.subTest(link=link.name):
                try:
                    link.symlink_to(target, target_is_directory=target.is_dir())
                except (OSError, NotImplementedError) as error:
                    self.skipTest(f"symlinks unavailable: {error}")
                with mock.patch.object(Path, "open", side_effect=AssertionError("symlink read")):
                    with self.assertRaises(baseline.BaselineError):
                        baseline.fingerprint_artifact(self.root, relative)
                self.assert_fingerprint_failure(relative)
                link.unlink()
        manifest = self.artifact.run_dir / "checksums.sha256"
        manifest.unlink()
        manifest.symlink_to(self.script)
        self.assert_fingerprint_failure()

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFO creation unavailable")
    def test_fifo_payload_and_inventory_are_rejected_without_open(self) -> None:
        for name in ("retained.fifo", "checksums.sha256"):
            path = self.artifact.run_dir / name
            if path.exists():
                path.unlink()
            os.mkfifo(path)
            with mock.patch.object(Path, "open", side_effect=AssertionError("FIFO open")):
                with self.assertRaises(baseline.BaselineError):
                    baseline.fingerprint_artifact(self.root, self.relative)
            self.assert_fingerprint_failure()
            path.unlink()

    @unittest.skipUnless(hasattr(socket, "AF_UNIX"), "Unix sockets unavailable")
    def test_socket_payload_is_rejected_without_open(self) -> None:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as retained:
            previous_cwd = Path.cwd()
            try:
                os.chdir(self.artifact.run_dir)
                retained.bind("retained.socket")  # Avoid platform sockaddr path-length limits.
            finally:
                os.chdir(previous_cwd)
            with mock.patch.object(Path, "open", side_effect=AssertionError("socket open")):
                with self.assertRaises(baseline.BaselineError):
                    baseline.fingerprint_artifact(self.root, self.relative)
            self.assert_fingerprint_failure()

    def test_full_and_legacy_reportless_artifacts_succeed(self) -> None:
        full = self.make_artifact(run_id="fingerprint-full", mode="bench-full")
        relative = full.run_dir.relative_to(self.root).as_posix()
        result = baseline.fingerprint_artifact(self.root, relative)
        self.assertEqual(result["source"]["mode"], "bench-full")
        for name in ("benchmark-report-v1.json", "benchmark-report-v1.md"):
            (full.run_dir / name).unlink()
        baseline.write_checksums(self.root, full.run_dir)
        reportless = baseline.fingerprint_artifact(self.root, relative)
        self.assertNotEqual(reportless["sha256"], result["sha256"])
        self.assertEqual(reportless["source"], result["source"])

    def test_failed_dirty_correctness_and_partial_artifacts_are_rejected(self) -> None:
        for run_id, mode, dirty, failed in (
            ("failed", "bench-smoke", False, True), ("dirty", "bench-smoke", True, False),
            ("correctness", "correctness", False, False),
        ):
            with self.subTest(case=run_id):
                run = self.make_artifact(run_id=run_id, mode=mode, dirty=dirty, failed=failed)
                self.assert_fingerprint_failure(run.run_dir.relative_to(self.root).as_posix())
        summary_path = self.artifact.run_dir / "summary.json"
        summary = json.loads(summary_path.read_text())
        summary["stress_samples"] = []
        summary["stress_aggregates"] = []
        baseline.write_json(summary_path, summary)
        baseline.write_checksums(self.root, self.artifact.run_dir)
        self.assertEqual(baseline.validate_artifact(self.root, self.artifact.run_dir), [])
        self.assert_fingerprint_failure()  # The copied complete report must not mask this.

    def test_symmetric_missing_full_target_and_missing_scenario_fail_closed(self) -> None:
        full = self.make_artifact(run_id="incomplete-full", mode="bench-full")
        relative = full.run_dir.relative_to(self.root).as_posix()
        summary_path = full.run_dir / "summary.json"
        original = json.loads(summary_path.read_text())
        summary = copy.deepcopy(original)
        summary["criterion_results"] = summary["criterion_results"][:1]
        baseline.write_json(summary_path, summary)
        baseline.write_json(full.run_dir / "criterion/parsed-estimates.json", summary["criterion_results"])
        baseline.write_checksums(self.root, full.run_dir)
        self.assertEqual(baseline.validate_artifact(self.root, full.run_dir), [])
        self.assert_fingerprint_failure(relative)
        summary = copy.deepcopy(original)
        first = summary["stress_samples"][0]
        summary["stress_samples"] = [item for item in summary["stress_samples"] if (
            item["operation"], item["in_flight"]
        ) != (first["operation"], first["in_flight"])]
        summary["stress_aggregates"] = [item for item in summary["stress_aggregates"] if (
            item["operation"], item["in_flight"]
        ) != (first["operation"], first["in_flight"])]
        baseline.write_json(summary_path, summary)
        baseline.write_json(full.run_dir / "criterion/parsed-estimates.json", original["criterion_results"])
        baseline.write_checksums(self.root, full.run_dir)
        self.assert_fingerprint_failure(relative)

    def test_missing_local_target_objects_fail_without_fetch(self) -> None:
        (self.root / ".git/objects" / self.sha[:2] / self.sha[2:]).unlink()
        self.assert_fingerprint_failure()

    def test_report_references_cannot_open_files_outside_the_named_artifact(self) -> None:
        run_dir = self.artifact.run_dir
        command = next(run_dir.glob("commands/*/command.json"))
        original_command = json.loads(command.read_text())
        summary_path = run_dir / "summary.json"
        original_summary = json.loads(summary_path.read_text())
        actual_open = Path.open

        def artifact_only_open(path, mode="r", *args, **kwargs):
            self.assertTrue(path.is_relative_to(run_dir), f"escaped read: {path}")
            return actual_open(path, mode, *args, **kwargs)

        for reference in (str(self.script), "scripts/baseline.py", self.relative + "/../../outside"):
            for field in ("stdout", "criterion"):
                with self.subTest(field=field, reference=reference):
                    changed_command = copy.deepcopy(original_command)
                    summary = copy.deepcopy(original_summary)
                    if field == "stdout":
                        changed_command["stdout_path"] = reference
                    else:
                        summary["criterion_results"][0]["source"] = reference
                    baseline.write_json(command, changed_command)
                    baseline.write_json(summary_path, summary)
                    baseline.write_json(run_dir / "criterion/parsed-estimates.json", summary["criterion_results"])
                    baseline.write_checksums(self.root, run_dir)
                    with mock.patch.object(Path, "open", artifact_only_open):
                        with self.assertRaises(baseline.BaselineError):
                            baseline.fingerprint_artifact(self.root, self.relative)
                    self.assert_fingerprint_failure()

    def test_malformed_source_input_and_io_errors_have_no_traceback(self) -> None:
        path = self.artifact.run_dir / "summary.json"
        for raw in (b"\xff", b"null", b'{"x":' * 1500 + b"0" + b"}" * 1500,
                    b'{"schema_version":' + b"9" * 5000 + b"}"):
            with self.subTest(raw=raw[:20]):
                path.write_bytes(raw)
                baseline.write_checksums(self.root, self.artifact.run_dir)
                self.assert_fingerprint_failure()
        for error in (PermissionError("denied"), FileNotFoundError("removed")):
            with mock.patch.object(Path, "open", side_effect=error):
                with self.assertRaises(baseline.BaselineError):
                    baseline.fingerprint_artifact(self.root, self.relative)

    def test_manifest_and_entry_limits_are_bounded_without_limiting_raw_logs(self) -> None:
        manifest = self.artifact.run_dir / "checksums.sha256"
        original = manifest.read_bytes()
        self.assertEqual(baseline.ARTIFACT_FINGERPRINT_MAX_CHECKSUM_BYTES, 4 * 1024 * 1024)
        self.assertEqual(baseline.ARTIFACT_FINGERPRINT_MAX_ENTRIES, 10_000)
        source = mock.MagicMock()
        source.__enter__.return_value.read.return_value = original
        with mock.patch.object(Path, "open", return_value=source):
            baseline._fingerprint_artifact_preflight(self.root, self.artifact.run_dir)
        source.__enter__.return_value.read.assert_called_once_with(4 * 1024 * 1024 + 1)
        with mock.patch.object(baseline, "ARTIFACT_FINGERPRINT_MAX_CHECKSUM_BYTES", len(original)):
            baseline.fingerprint_artifact(self.root, self.relative)
        with mock.patch.object(baseline, "ARTIFACT_FINGERPRINT_MAX_CHECKSUM_BYTES", len(original) - 1):
            with self.assertRaisesRegex(baseline.BaselineError, "4 MiB"):
                baseline.fingerprint_artifact(self.root, self.relative)
        manifest.write_bytes(b" " * (4 * 1024 * 1024 + 1))
        with mock.patch.object(baseline, "build_benchmark_report", side_effect=AssertionError("oversize")):
            with self.assertRaises(baseline.BaselineError):
                baseline.fingerprint_artifact(self.root, self.relative)
        self.assert_fingerprint_failure()
        manifest.write_bytes(original)
        count = len(list(self.artifact.run_dir.rglob("*")))
        with mock.patch.object(baseline, "ARTIFACT_FINGERPRINT_MAX_ENTRIES", count):
            baseline.fingerprint_artifact(self.root, self.relative)
        with mock.patch.object(baseline, "ARTIFACT_FINGERPRINT_MAX_ENTRIES", count - 1):
            with self.assertRaisesRegex(baseline.BaselineError, "entry limit"):
                baseline.fingerprint_artifact(self.root, self.relative)
        log = self.artifact.run_dir / "large.raw.log"
        with log.open("wb") as target:
            for _ in range(5):
                target.write(b"x" * (1024 * 1024))
        baseline.write_checksums(self.root, self.artifact.run_dir)
        actual_open = Path.open

        def streaming_open(path, mode="r", *args, **kwargs):
            source = actual_open(path, mode, *args, **kwargs)
            if path == log:
                proxy = mock.MagicMock(wraps=source)
                proxy.__enter__.return_value = proxy
                proxy.__exit__.side_effect = lambda *unused: source.close()

                def bounded_read(size=-1):
                    self.assertGreater(size, 0)
                    self.assertLessEqual(size, 1024 * 1024)
                    return source.read(size)

                proxy.read.side_effect = bounded_read
                return proxy
            return source

        with mock.patch.object(Path, "open", streaming_open):
            result = baseline.fingerprint_artifact(self.root, self.relative)
        self.assertIn("large.raw.log", [item["path"] for item in result["content"]["files"]])

    def test_only_read_only_local_git_and_artifact_reads_are_used(self) -> None:
        full = self.make_artifact(run_id="local-only-full", mode="bench-full")
        relative = full.run_dir.relative_to(self.root).as_posix()
        actual_run = subprocess.run
        calls = []

        def local_git(argv, **kwargs):
            self.assertEqual(tuple(argv[:5]), ("git", "--no-lazy-fetch", "--no-replace-objects", "cat-file", "blob"))
            self.assertIn(argv[5], (f"{self.sha}:Cargo.lock", f"{self.sha}:benchmarks/Cargo.toml"))
            calls.append(argv[5])
            return actual_run(argv, **kwargs)

        actual_open = Path.open

        def read_only_open(path, mode="r", *args, **kwargs):
            self.assertIn(mode, ("r", "rb"))
            self.assertTrue(path.is_relative_to(full.run_dir), f"escaped read: {path}")
            return actual_open(path, mode, *args, **kwargs)

        before = self.inventory()
        output = io.BytesIO()
        stdout = io.TextIOWrapper(output, encoding="utf-8")
        stderr = io.StringIO()
        with contextlib.ExitStack() as stack:
            for name in ("bootstrap_repository", "run_mode", "run_benchmarks", "collect_environment",
                         "write_json", "write_checksums", "load_policy_file", "load_controlled_evidence_contract_file"):
                stack.enter_context(mock.patch.object(baseline, name, side_effect=AssertionError(name)))
            for name in ("socket.create_connection", "urllib.request.urlopen"):
                stack.enter_context(mock.patch(name, side_effect=AssertionError(name)))
            stack.enter_context(mock.patch.object(baseline.subprocess, "run", local_git))
            stack.enter_context(mock.patch.object(Path, "open", read_only_open))
            stack.enter_context(mock.patch.object(baseline, "__file__", str(self.script)))
            stack.enter_context(contextlib.redirect_stdout(stdout))
            stack.enter_context(contextlib.redirect_stderr(stderr))
            result = baseline.fingerprint_artifact(self.root, relative)
            self.assertEqual(baseline.main(["fingerprint-artifact", relative]), 0)
        self.assertEqual(len(calls), 4)
        self.assertEqual(json.loads(output.getvalue()), result)
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(result["qualification"], baseline.ARTIFACT_FINGERPRINT_QUALIFICATION)
        self.assertEqual(self.inventory(), before)

    def test_cli_help_and_usage_exit_contract(self) -> None:
        before = self.inventory()
        for args in (("--help",), ("fingerprint-artifact", "--help")):
            result = self.cli(*args)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("fingerprint-artifact", result.stdout)
            self.assertEqual(result.stderr, "")
        for args in (("fingerprint-artifact",), ("fingerprint-artifact", self.relative, "--latest"),
                     ("fingerprint-artifact", self.relative, "extra")):
            result = self.cli(*args)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertEqual(result.stdout, "")
            self.assertIn("usage:", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(self.inventory(), before)

    def test_cli_emits_utf8_bytes_even_with_an_ascii_stdout_locale(self) -> None:
        (self.artifact.run_dir / "évidence.txt").write_bytes(b"retained")
        baseline.write_checksums(self.root, self.artifact.run_dir)
        result = subprocess.run(
            [sys.executable, str(self.script), "fingerprint-artifact", self.relative],
            cwd=self.root.parent, capture_output=True, timeout=15,
            env={**os.environ, "PYTHONIOENCODING": "ascii"},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, b"")
        self.assertIn('évidence.txt'.encode("utf-8"), result.stdout)
        self.assertNotIn(b"\r\n", result.stdout)
        self.assertEqual(json.loads(result.stdout)["source"]["run_id"], self.artifact.run_id)


class ControlledArtifactBindingTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve() / "repository"
        self.root.mkdir()
        self.sha = initialize_git_lock_fixture(self.root)
        (self.root / "scripts").mkdir()
        self.script = self.root / "scripts/baseline.py"
        shutil.copyfile(SCRIPTS / "baseline.py", self.script)
        self.contract_relative = "inputs/contrôle evidence.json"
        self.bindings_relative = "inputs/artifact bindings-é.json"
        (self.root / "inputs").mkdir()
        self.contract_path = self.root / self.contract_relative
        self.bindings_path = self.root / self.bindings_relative
        self.policy = self.root / "policy.json"
        shutil.copyfile(ROOT / "benchmarks/policy/benchmark-budget-policy-v1.json", self.policy)
        self.contract = controlled_evidence_contract_fixture()
        # Keep one two-run study and its synthetic approved baseline. Other retained
        # evidence is deliberately still opaque, unreferenced or non-artifact data.
        self.contract["variance_studies"] = self.contract["variance_studies"][1:]
        self.contract["baselines"] = self.contract["baselines"][1:]
        self.contract["baselines"][0]["target_sha"] = self.sha
        self.study = self.contract["variance_studies"][0]
        self.study["target_sha"] = self.sha
        self.artifacts = {}
        self.expected_bindings = []
        self.bindings: dict = {
            "binding_schema": {"name": "benchmark-controlled-artifact-bindings", "version": 1},
            "contract_sha256": "0" * 64,
            "artifact_content_schema": {"name": "benchmark-artifact-content", "version": 1},
            "artifacts": [],
        }
        for run in self.study["runs"]:
            run["target_sha"] = self.sha
            artifact = self.make_artifact(run["run_id"])
            evidence_id = run["artifact_evidence_id"]
            digest = self.independent_content_sha256(artifact.run_dir)
            self.artifacts[evidence_id] = artifact
            next(item for item in self.contract["evidence_retention"] if item["evidence_id"] == evidence_id)["sha256"] = digest
            self.bindings["artifacts"].append({
                "evidence_id": evidence_id, "run_dir": artifact.run_dir.relative_to(self.root).as_posix(),
            })
            self.expected_bindings.append({
                "evidence_id": evidence_id, "study_id": self.study["study_id"],
                "target_sha": self.sha, "run_id": run["run_id"], "mode": "bench-full",
                "content_sha256": digest,
            })
        self.refresh_contract_pin()

    def make_artifact(self, run_id: str, mode: str = "bench-full") -> baseline.ArtifactRun:
        artifact = baseline.ArtifactRun(
            repo_root=self.root, output_root=self.root / "retained espace-é",
            target_sha=self.sha, run_id=run_id, mode=mode,
            runner_label="synthetic-not-proof-of-control", dirty=False, allow_dirty=False,
        )
        artifact.create()
        populate_benchmark_evidence(artifact)
        artifact.finalize()
        return artifact

    def independent_content_sha256(self, run_dir: Path) -> str:
        files = [
            {"path": path.relative_to(run_dir).as_posix(),
             "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for path in run_dir.rglob("*")
            if path.is_file() and path != run_dir / "checksums.sha256"
        ]
        content = {"content_schema": {"name": "benchmark-artifact-content", "version": 1},
                   "files": sorted(files, key=lambda item: item["path"].encode("utf-8"))}
        return hashlib.sha256((json.dumps(
            content, sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False
        ) + "\n").encode("utf-8")).hexdigest()

    def write_documents(self) -> None:
        self.contract_path.write_text(json.dumps(self.contract), encoding="utf-8")
        self.bindings_path.write_text(json.dumps(self.bindings), encoding="utf-8")

    def refresh_contract_pin(self) -> None:
        self.contract["approval"]["scope_sha256"] = (
            baseline._controlled_evidence_contract_approval_scope_sha256(self.contract)
        )
        self.assertEqual(baseline.validate_controlled_evidence_contract(self.contract), [])
        self.bindings["contract_sha256"] = baseline.controlled_evidence_contract_sha256(self.contract)
        self.write_documents()

    def inventory(self) -> dict:
        return {
            path.relative_to(self.root).as_posix(): (
                ("symlink", str(path.readlink())) if path.is_symlink()
                else ("file", path.read_bytes()) if path.is_file()
                else ("directory", None) if path.is_dir()
                else ("special", None)
            )
            for path in self.root.rglob("*")
        }

    def cli(self, *args: str, ascii_stdout: bool = False) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            [sys.executable, str(self.script), *args], cwd=self.root.parent,
            capture_output=True, timeout=20,
            env={**os.environ, **({"PYTHONIOENCODING": "ascii"} if ascii_stdout else {})},
        )

    def verify(self) -> dict:
        return baseline.verify_controlled_artifacts(self.root, self.contract_relative, self.bindings_relative)

    def assert_verification_scope(self, result: dict) -> None:
        self.assertEqual(result["verification_schema"], {
            "name": "benchmark-controlled-artifact-verification", "version": 1,
        })
        self.assertEqual(result["verification_scope"], "variance_run_artifact_content_and_declared_run_identity_only")
        self.assertEqual(result["qualification"], "integrity_only_not_authentication_or_owner_authorization")
        self.assertEqual(result["performance_enforcement"], {
            "state": "not_eligible", "reason": "artifact_binding_verification_only",
        })
        self.assertEqual(result["not_verified"], [
            "producer_set_sha256", "scenario_set_sha256", "budget_scenario_identity_sha256",
            "runner_profile_control_and_environment_equality", "statistical_method_and_variance_analysis",
            "independent_executions", "non_artifact_and_unmapped_retained_evidence",
            "expiration_and_continued_retention", "approval_authentication_and_owner_authorization",
            "baseline_acceptance", "performance_enforcement",
        ])
        self.assertNotIn('"approved"', json.dumps(result))
        self.assertNotIn('"files"', json.dumps(result))

    def test_complete_bindings_match_independent_hashes_and_identity_read_only(self) -> None:
        before = self.inventory()
        result = self.verify()
        self.assertEqual(result["verified_artifacts"], self.expected_bindings)
        self.assertEqual(result["contract"], {
            "contract_id": self.contract["contract_id"], "canonical_sha256": self.bindings["contract_sha256"],
        })
        canonical = copy.deepcopy(self.bindings)
        canonical["artifacts"].sort(key=lambda item: item["evidence_id"])
        digest = hashlib.sha256((json.dumps(
            canonical, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False
        ) + "\n").encode("utf-8")).hexdigest()
        self.assertEqual(result["binding_manifest"], {
            "schema": self.bindings["binding_schema"], "canonical_sha256": digest,
        })
        self.assert_verification_scope(result)
        self.assertEqual(self.inventory(), before)

    def test_cli_is_one_canonical_utf8_result_from_alternate_cwd(self) -> None:
        before = self.inventory()
        result = self.cli("verify-controlled-artifacts", self.contract_relative, self.bindings_relative, ascii_stdout=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, b"")
        document = json.loads(result.stdout)
        self.assertEqual(document["verified_artifacts"], self.expected_bindings)
        self.assert_verification_scope(document)
        self.assertEqual(result.stdout, (json.dumps(
            document, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False
        ) + "\n").encode("utf-8"))
        self.assertEqual(self.inventory(), before)

    def assert_failure(
        self, *, before_artifacts: bool = False,
        contract_relative: str | None = None, bindings_relative: str | None = None,
    ) -> None:
        contract_relative = self.contract_relative if contract_relative is None else contract_relative
        bindings_relative = self.bindings_relative if bindings_relative is None else bindings_relative
        before = self.inventory()
        with contextlib.ExitStack() as stack:
            if before_artifacts:
                stack.enter_context(mock.patch.object(
                    baseline, "fingerprint_artifact", side_effect=AssertionError("premature fingerprint")
                ))
                stack.enter_context(mock.patch.object(
                    baseline, "_sha256", side_effect=AssertionError("premature payload hash")
                ))
                stack.enter_context(mock.patch.object(
                    baseline.subprocess, "run", side_effect=AssertionError("premature Git")
                ))
            with self.assertRaises(baseline.BaselineError):
                baseline.verify_controlled_artifacts(self.root, contract_relative, bindings_relative)
        result = self.cli("verify-controlled-artifacts", contract_relative, bindings_relative)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, b"")
        self.assertTrue(result.stderr.startswith(b"baseline: "), result.stderr)
        self.assertNotIn(b"Traceback", result.stderr)
        self.assertNotIn(b"sensitive-document-value", result.stderr)
        self.assertEqual(self.inventory(), before)

    def test_contract_and_mapping_permutations_preserve_pins_and_result(self) -> None:
        first = self.verify()
        original_manifest = baseline.controlled_artifact_bindings_json_text(self.bindings)
        self.bindings["artifacts"].reverse()
        self.contract["evidence_retention"].reverse()
        self.contract["budget_rules"].reverse()
        self.contract["control"]["runner"]["evidence_ids"].reverse()
        self.study["runs"].reverse()
        self.write_documents()  # Deliberately do not recompute either pin.
        self.assertEqual(baseline.controlled_artifact_bindings_json_text(self.bindings), original_manifest)
        self.assertEqual(self.verify(), first)
        result = self.cli("verify-controlled-artifacts", self.contract_relative, self.bindings_relative)
        self.assertEqual(json.loads(result.stdout), first)

    def test_stale_raw_and_approval_scope_pins_fail_before_artifact_work(self) -> None:
        original_pin = self.bindings["contract_sha256"]
        for wrong_pin in (
            hashlib.sha256(self.contract_path.read_bytes()).hexdigest(),
            self.contract["approval"]["scope_sha256"], "f" * 64,
        ):
            with self.subTest(pin=wrong_pin):
                self.assertNotEqual(wrong_pin, original_pin)
                self.bindings["contract_sha256"] = wrong_pin
                self.write_documents()
                self.assert_failure(before_artifacts=True)
        self.bindings["contract_sha256"] = original_pin
        self.contract["approval"]["approved_utc"] = "2026-03-01T00:00:00Z"
        self.assertEqual(baseline.validate_controlled_evidence_contract(self.contract), [])
        self.write_documents()
        self.assert_failure(before_artifacts=True)

    def test_manifest_schema_is_exact_and_rejects_duplicates_and_unsupported_schemes(self) -> None:
        original = copy.deepcopy(self.bindings)
        cases = [None, [], {}, {key: value for key, value in original.items() if key != "contract_sha256"}]
        for path, value in (
            (("binding_schema", "name"), "sensitive-document-value"),
            (("binding_schema", "version"), True), (("binding_schema", "version"), 2),
            (("artifact_content_schema", "name"), "report-only"),
            (("artifact_content_schema", "version"), True), (("artifact_content_schema", "version"), 2),
            (("artifact_content_schema", "extra"), "sensitive-document-value"),
            (("contract_sha256",), "A" * 64), (("contract_sha256",), "short"),
            (("contract_sha256",), False), (("artifacts",), []), (("artifacts",), {}),
            (("artifacts", 0, "evidence_id"), "bad ID"),
            (("artifacts", 0, "actual_sha256"), "0" * 64),
            (("sensitive-document-value",), "sensitive-document-value"),
        ):
            changed = copy.deepcopy(original)
            target = changed
            for part in path[:-1]:
                target = target[part]
            target[path[-1]] = value
            cases.append(changed)
        duplicate_id = copy.deepcopy(original)
        duplicate_id["artifacts"][1]["evidence_id"] = duplicate_id["artifacts"][0]["evidence_id"]
        duplicate_path = copy.deepcopy(original)
        duplicate_path["artifacts"][1]["run_dir"] = duplicate_path["artifacts"][0]["run_dir"]
        cases.extend((duplicate_id, duplicate_path))
        for position, document in enumerate(cases):
            with self.subTest(case=position):
                self.bindings_path.write_text(json.dumps(document), encoding="utf-8")
                self.assert_failure(before_artifacts=True)

    def test_mapping_coverage_is_exact_not_a_partial_or_retention_selection(self) -> None:
        original = copy.deepcopy(self.bindings)
        for replacement in (None, "unknown-evidence", "evidence-approval-record", "evidence-run-old-a"):
            with self.subTest(replacement=replacement):
                self.bindings = copy.deepcopy(original)
                if replacement is None:
                    self.bindings["artifacts"].pop()
                else:
                    self.bindings["artifacts"][1]["evidence_id"] = replacement
                self.write_documents()
                self.assert_failure(before_artifacts=True)
        self.bindings = copy.deepcopy(original)
        self.bindings["artifacts"].append({"evidence_id": "evidence-run-old-a", "run_dir": "unmapped"})
        self.write_documents()
        self.assert_failure(before_artifacts=True)

    def test_invalid_contract_schema_lifecycle_approval_and_kind_are_delegated(self) -> None:
        original = copy.deepcopy(self.contract)
        for path, value in (
            (("contract_schema", "version"), 2),
            (("baseline_lifecycle", "promotion_path"), []),
            (("baselines", 0, "promotion_chain"), []),
            (("approval", "scope_sha256"), "0" * 64),
            (("variance_studies", 0, "runs", 1, "artifact_evidence_id"), "evidence-approval-record"),
        ):
            with self.subTest(path=path):
                self.contract = copy.deepcopy(original)
                target = self.contract
                for part in path[:-1]:
                    target = target[part]
                target[path[-1]] = value
                self.write_documents()
                with mock.patch.object(
                    baseline, "validate_controlled_evidence_contract", wraps=baseline.validate_controlled_evidence_contract
                ) as validate:
                    self.assert_failure(before_artifacts=True)
                    validate.assert_called()

    def test_manifest_json_encoding_duplicates_nonfinite_and_deep_values_are_rejected(self) -> None:
        raw = self.bindings_path.read_bytes()
        cases = [
            b"\xff", b"", b'{"sensitive-document-value":', raw + b"null",
            raw.replace(b'"contract_sha256":', b'"contract_sha256":"duplicate","contract_sha256":'),
            raw.replace(b'"version": 1', b'"version":1,"version":1', 1),
            b'{"x":' * 65 + b"0" + b"}" * 65,
            raw.replace(b"retained espace-", b"retained \\ud800-", 1),
        ]
        cases.extend(raw.replace(b'"version": 1', b'"version": ' + token, 1) for token in (
            b"NaN", b"Infinity", b"-Infinity", b"1e9999", b"9" * 5000,
        ))
        for position, invalid in enumerate(cases):
            with self.subTest(case=position):
                self.bindings_path.write_bytes(invalid)
                self.assert_failure(before_artifacts=True)

    def test_manifest_byte_depth_and_mapping_limits_are_local_and_bounded(self) -> None:
        raw = self.bindings_path.read_bytes()
        self.assertEqual(baseline.CONTROLLED_ARTIFACT_BINDINGS_MAX_BYTES, 1024 * 1024)
        source = mock.MagicMock()
        source.__enter__.return_value.read.return_value = raw
        with mock.patch.object(Path, "open", return_value=source):
            baseline.load_controlled_artifact_bindings_file(self.root, self.bindings_relative)
        source.__enter__.return_value.read.assert_called_once_with(1024 * 1024 + 1)
        self.bindings_path.write_bytes(raw + b" " * (1024 * 1024 - len(raw)))
        self.assertEqual(self.verify()["verified_artifacts"], self.expected_bindings)
        with self.bindings_path.open("ab") as output:
            output.write(b" ")
        with mock.patch.object(baseline.json, "loads", side_effect=AssertionError("oversize parse")):
            with self.assertRaises(baseline.BaselineError):
                baseline.load_controlled_artifact_bindings_file(self.root, self.bindings_relative)
        self.assert_failure(before_artifacts=True)
        self.bindings_path.write_bytes(b"[" * 65 + b"0" + b"]" * 65)
        with mock.patch.object(baseline.json, "loads", side_effect=AssertionError("deep parse")):
            with self.assertRaises(baseline.BaselineError):
                baseline.load_controlled_artifact_bindings_file(self.root, self.bindings_relative)
        self.assertEqual(baseline.CONTROLLED_ARTIFACT_BINDINGS_MAX_ARTIFACTS, 128)
        self.bindings["artifacts"] = [
            {"evidence_id": f"evidence-{number:03}", "run_dir": f"not-opened/{number}"} for number in range(128)
        ]
        self.write_documents()
        loaded = baseline.load_controlled_artifact_bindings_file(self.root, self.bindings_relative)
        self.assertEqual(len(loaded["artifacts"]), 128)
        self.bindings["artifacts"].append({"evidence_id": "evidence-128", "run_dir": "not-opened/128"})
        self.write_documents()
        self.assert_failure(before_artifacts=True)

    def test_all_directory_paths_are_preflighted_including_later_invalid_mapping(self) -> None:
        original = self.bindings["artifacts"][1]["run_dir"]
        for path in ("", ".", "../outside", "./" + original, str(self.root / original), "C:/artifact",
                     original + "/", original.replace("/", "//", 1), original.replace("/", "\\", 1),
                     "missing-directory", "policy.json", "bad\x00path"):
            with self.subTest(path=path):
                self.bindings["artifacts"][1]["run_dir"] = path
                self.write_documents()
                self.assert_failure(before_artifacts=True)
        link = self.root / "artifact-link"
        try:
            link.symlink_to(self.root / original, target_is_directory=True)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        self.bindings["artifacts"][1]["run_dir"] = "artifact-link"
        self.write_documents()
        self.assert_failure(before_artifacts=True)
        link.unlink()
        link.symlink_to((self.root / original).parent, target_is_directory=True)
        self.bindings["artifacts"][1]["run_dir"] = "artifact-link/" + (self.root / original).name
        self.write_documents()
        self.assert_failure(before_artifacts=True)

    def test_same_local_directory_aliases_fail_before_fingerprinting(self) -> None:
        first = self.bindings["artifacts"][0]["run_dir"]
        alias = first.replace("retained", "RETAINED", 1)
        if not (self.root / alias).exists() or not (self.root / alias).samefile(self.root / first):
            self.skipTest("case-insensitive directory alias unavailable on this filesystem")
        self.bindings["artifacts"][1]["run_dir"] = alias
        self.write_documents()
        self.assert_failure(before_artifacts=True)

    def test_both_document_inputs_require_explicit_regular_nonsymlink_files(self) -> None:
        for field, path in (("contract_relative", self.contract_path), ("bindings_relative", self.bindings_path)):
            for invalid in (str(path), "../outside.json", "./" + path.relative_to(self.root).as_posix(),
                            "", "inputs", "missing.json"):
                with self.subTest(field=field, path=invalid):
                    self.assert_failure(before_artifacts=True, **{field: invalid})
        link = self.root / "document-link"
        try:
            link.symlink_to(self.bindings_path)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        self.assert_failure(before_artifacts=True, bindings_relative="document-link")
        link.unlink()
        link.symlink_to(self.contract_path)
        self.assert_failure(before_artifacts=True, contract_relative="document-link")
        link.unlink()
        link.symlink_to(self.root / "inputs", target_is_directory=True)
        self.assert_failure(before_artifacts=True, bindings_relative="document-link/" + self.bindings_path.name)
        self.assert_failure(before_artifacts=True, contract_relative="document-link/" + self.contract_path.name)

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFO creation unavailable")
    def test_document_and_directory_fifos_are_rejected_without_opening(self) -> None:
        fifo = self.root / "fifo"
        os.mkfifo(fifo)
        actual_open = Path.open

        def no_fifo_open(path, *args, **kwargs):
            self.assertNotEqual(path, fifo)
            return actual_open(path, *args, **kwargs)

        with mock.patch.object(Path, "open", no_fifo_open):
            self.assert_failure(before_artifacts=True, contract_relative="fifo")
            self.assert_failure(before_artifacts=True, bindings_relative="fifo")
            self.bindings["artifacts"][1]["run_dir"] = "fifo"
            self.write_documents()
            self.assert_failure(before_artifacts=True)

    def test_later_tampering_and_fresh_checksums_never_emit_partial_success(self) -> None:
        artifact = self.artifacts["evidence-run-current-b"]
        raw = next(artifact.run_dir.glob("commands/*/command.stderr"))
        raw.write_bytes(b"new retained raw evidence")
        self.assert_failure()
        baseline.write_checksums(self.root, artifact.run_dir)
        fingerprint = baseline.fingerprint_artifact(self.root, artifact.run_dir.relative_to(self.root).as_posix())
        self.assertNotEqual(fingerprint["sha256"], self.expected_bindings[1]["content_sha256"])
        self.assert_failure()

    def test_swapped_directories_and_wrong_declared_target_sha_are_rejected(self) -> None:
        a, b = self.bindings["artifacts"]
        a["run_dir"], b["run_dir"] = b["run_dir"], a["run_dir"]
        self.write_documents()
        self.assert_failure()
        a["run_dir"], b["run_dir"] = b["run_dir"], a["run_dir"]
        self.study["target_sha"] = "b" * 40
        for run in self.study["runs"]:
            run["target_sha"] = "b" * 40
        self.contract["baselines"][0]["target_sha"] = "b" * 40
        self.refresh_contract_pin()
        self.assert_failure()

    def test_valid_smoke_fingerprint_is_not_a_full_variance_run(self) -> None:
        smoke = self.make_artifact("synthetic-smoke", mode="bench-smoke")
        self.study["runs"][1]["run_id"] = smoke.run_id
        self.bindings["artifacts"][1]["run_dir"] = smoke.run_dir.relative_to(self.root).as_posix()
        next(item for item in self.contract["evidence_retention"] if item["evidence_id"] == "evidence-run-current-b")["sha256"] = self.independent_content_sha256(smoke.run_dir)
        self.refresh_contract_pin()
        self.assertEqual(baseline.fingerprint_artifact(self.root, self.bindings["artifacts"][1]["run_dir"])["source"]["mode"], "bench-smoke")
        self.assert_failure()

    def test_incomplete_dirty_and_unsupported_artifacts_inherit_fingerprint_rejection(self) -> None:
        artifact = self.artifacts["evidence-run-current-b"]
        summary_path = artifact.run_dir / "summary.json"
        provenance_path = artifact.run_dir / "provenance.json"
        original_summary = json.loads(summary_path.read_text())
        original_provenance = json.loads(provenance_path.read_text())
        for kind in ("incomplete", "dirty", "unsupported"):
            with self.subTest(kind=kind):
                summary, provenance = copy.deepcopy(original_summary), copy.deepcopy(original_provenance)
                if kind == "incomplete":
                    summary["stress_samples"], summary["stress_aggregates"] = [], []
                elif kind == "dirty":
                    summary.update(status="invalid", baseline_valid=False, invalid_reasons=["synthetic dirty"])
                    provenance.update(dirty=True, dirty_override=True, baseline_eligible=False)
                else:
                    provenance["harness_version"] = "unsupported"
                baseline.write_json(summary_path, summary)
                baseline.write_json(provenance_path, provenance)
                baseline.write_checksums(self.root, artifact.run_dir)
                self.assert_failure()  # An untouched copied complete report cannot substitute.

    def test_missing_local_git_objects_fail_without_a_weaker_fallback(self) -> None:
        (self.root / ".git/objects" / self.sha[:2] / self.sha[2:]).unlink()
        self.assert_failure()

    def test_duplicate_actual_content_and_scheme_drift_are_rejected_defensively(self) -> None:
        responses = [
            {"content": {"content_schema": self.bindings["artifact_content_schema"]},
             "sha256": row["content_sha256"], "source": {key: row[key] for key in ("mode", "run_id", "target_sha")}}
            for row in self.expected_bindings
        ]
        responses[1]["sha256"] = responses[0]["sha256"]
        with mock.patch.object(baseline, "fingerprint_artifact", side_effect=responses):
            with self.assertRaisesRegex(baseline.BaselineError, "distinct"):
                self.verify()
        responses[0]["content"] = {"content_schema": {"name": "benchmark-artifact-content", "version": 2}}
        with mock.patch.object(baseline, "fingerprint_artifact", return_value=responses[0]):
            with self.assertRaisesRegex(baseline.BaselineError, "scheme"):
                self.verify()

    def test_fingerprints_are_sequential_sorted_and_full_inventories_are_released(self) -> None:
        class Fingerprint(dict):
            pass

        original = baseline.fingerprint_artifact
        previous = None
        visited = []

        def fingerprint(root, path):
            nonlocal previous
            if previous is not None:
                self.assertIsNone(previous(), "previous full inventory retained")
            result = Fingerprint(original(root, path))
            previous = weakref.ref(result)
            visited.append(path)
            return result

        self.bindings["artifacts"].reverse()
        self.write_documents()
        with mock.patch.object(baseline, "fingerprint_artifact", fingerprint):
            self.verify()
        self.assertEqual(visited, [item["run_dir"] for item in sorted(self.bindings["artifacts"], key=lambda item: item["evidence_id"])])
        self.assertIsNotNone(previous)
        if previous is not None:
            self.assertIsNone(previous())

    def test_non_artifact_evidence_expiration_and_opaque_digests_remain_unverified(self) -> None:
        for item in self.contract["evidence_retention"]:
            item["locator"] = "opaque:unresolvable/" + item["evidence_id"]
            item["recorded_utc"] = "2020-01-01T00:00:00Z"
            item["retained_until_utc"] = "2021-01-01T00:00:00Z"
        self.contract["approval"]["approved_utc"] = "2020-06-01T00:00:00Z"
        self.study["producer_set_sha256"] = "c" * 64
        self.study["scenario_set_sha256"] = "d" * 64
        for run in self.study["runs"]:
            run["producer_set_sha256"], run["scenario_set_sha256"] = "c" * 64, "d" * 64
        self.contract["budget_rules"][0]["scenario_identity"]["identity_sha256"] = "e" * 64
        self.refresh_contract_pin()
        self.assert_verification_scope(self.verify())

    def test_verifier_and_cli_only_read_named_inputs_artifacts_and_local_git_objects(self) -> None:
        actual_run, actual_open = subprocess.run, Path.open
        calls = []

        def local_git(argv, **kwargs):
            self.assertEqual(tuple(argv[:5]), ("git", "--no-lazy-fetch", "--no-replace-objects", "cat-file", "blob"))
            self.assertIn(argv[5], (f"{self.sha}:Cargo.lock", f"{self.sha}:benchmarks/Cargo.toml"))
            calls.append(argv[5])
            return actual_run(argv, **kwargs)

        def read_only_open(path, mode="r", *args, **kwargs):
            self.assertIn(mode, ("r", "rb"))
            self.assertTrue(
                path in (self.contract_path, self.bindings_path)
                or any(path.is_relative_to(artifact.run_dir) for artifact in self.artifacts.values()),
                f"unexpected evidence/locator/policy read: {path}",
            )
            return actual_open(path, mode, *args, **kwargs)

        before = self.inventory()
        output, stderr = io.BytesIO(), io.StringIO()
        stdout = io.TextIOWrapper(output, encoding="utf-8")
        with contextlib.ExitStack() as stack:
            for name in ("bootstrap_repository", "run_mode", "run_benchmarks", "collect_environment",
                         "write_json", "write_checksums", "load_policy_file", "controlled_evaluate_artifacts",
                         "utc_now"):
                stack.enter_context(mock.patch.object(baseline, name, side_effect=AssertionError(name)))
            for name in ("socket.create_connection", "urllib.request.urlopen"):
                stack.enter_context(mock.patch(name, side_effect=AssertionError(name)))
            stack.enter_context(mock.patch.object(baseline.subprocess, "run", local_git))
            stack.enter_context(mock.patch.object(Path, "open", read_only_open))
            stack.enter_context(mock.patch.object(baseline, "__file__", str(self.script)))
            stack.enter_context(contextlib.redirect_stdout(stdout))
            stack.enter_context(contextlib.redirect_stderr(stderr))
            result = self.verify()
            self.assertEqual(baseline.main(["verify-controlled-artifacts", self.contract_relative, self.bindings_relative]), 0)
        self.assertEqual(len(calls), 8)
        self.assertEqual(json.loads(output.getvalue()), result)
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(self.inventory(), before)

    def test_binding_read_errors_are_baseline_errors_without_cli_traceback(self) -> None:
        actual_open = Path.open

        def denied(path, *args, **kwargs):
            if path == self.bindings_path:
                raise PermissionError("sensitive-document-value")
            return actual_open(path, *args, **kwargs)

        before = self.inventory()
        output, stderr = io.BytesIO(), io.StringIO()
        stdout = io.TextIOWrapper(output, encoding="utf-8")
        with mock.patch.object(Path, "open", denied), mock.patch.object(baseline, "__file__", str(self.script)), \
                contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            with self.assertRaises(baseline.BaselineError):
                self.verify()
            self.assertEqual(baseline.main(["verify-controlled-artifacts", self.contract_relative, self.bindings_relative]), 1)
        self.assertEqual(output.getvalue(), b"")
        self.assertNotIn("Traceback", stderr.getvalue())
        self.assertNotIn("sensitive-document-value", stderr.getvalue())
        self.assertEqual(self.inventory(), before)

    def test_cli_help_missing_arguments_and_unknown_flags(self) -> None:
        for args in (("--help",), ("verify-controlled-artifacts", "--help")):
            result = self.cli(*args)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(b"verify-controlled-artifacts", result.stdout)
            self.assertEqual(result.stderr, b"")
        for args in (("verify-controlled-artifacts",), ("verify-controlled-artifacts", self.contract_relative),
                     ("verify-controlled-artifacts", self.contract_relative, self.bindings_relative, "--latest")):
            result = self.cli(*args)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertEqual(result.stdout, b"")
            self.assertIn(b"usage:", result.stderr)


class SetIdentityTests(unittest.TestCase):
    def producers(self) -> list[dict]:
        return [
            {"adapter": "β", "id": "b", "producer": "other", "version": "2"},
            {"adapter": "A", "id": "a", "producer": "tool", "version": "1"},
        ]

    def scenarios(self) -> list[dict]:
        return [
            {"kind": "tcp_stress", "producer_id": "rusty-modbus-stress-json-v1", "identity": {
                "clients": 1, "duration_seconds": 5, "in_flight": 8, "operation": "read",
                "registers": 10, "repetitions": 5, "transport": "tcp", "warmup_seconds": 1,
            }},
            {"kind": "criterion_estimate", "producer_id": "criterion-0.5.1-private-estimates-layout",
             "identity": {"benchmark_id": "codec/decode"}},
        ]

    def test_producer_set_known_preimage_and_independent_openssl_digest(self) -> None:
        expected = (
            '{"producers":[{"adapter":"A","id":"a","producer":"tool","version":"1"},'
            '{"adapter":"β","id":"b","producer":"other","version":"2"}],'
            '"set_schema":{"name":"benchmark-producer-set","version":1}}\n'
        )
        result = baseline.producer_set_identity(self.producers())
        self.assertEqual(baseline.artifact_fingerprint_json_text(result["preimage"]), expected)
        self.assertEqual(result["sha256"], "d7a54f4d353a92f0e37f78bb610c46b07c05d6d437479609275f775c225c1f38")

    def test_scenario_set_known_preimage_and_independent_openssl_digest(self) -> None:
        expected = (
            '{"scenarios":[{"identity":{"benchmark_id":"codec/decode"},"kind":"criterion_estimate",'
            '"producer_id":"criterion-0.5.1-private-estimates-layout"},'
            '{"identity":{"clients":1,"duration_seconds":5,"in_flight":8,"operation":"read",'
            '"registers":10,"repetitions":5,"transport":"tcp","warmup_seconds":1},'
            '"kind":"tcp_stress","producer_id":"rusty-modbus-stress-json-v1"}],'
            '"set_schema":{"name":"benchmark-scenario-set","version":1}}\n'
        )
        result = baseline.scenario_set_identity(self.scenarios())
        self.assertEqual(baseline.artifact_fingerprint_json_text(result["preimage"]), expected)
        self.assertEqual(result["sha256"], "f34c95968055803ee6753dca4f433da4afe879adcf6cd976de0a15f7dcc16fb1")


    def test_sets_are_order_independent_and_detached_from_inputs(self) -> None:
        for helper, records in ((baseline.producer_set_identity, self.producers()),
                                (baseline.scenario_set_identity, self.scenarios())):
            original = copy.deepcopy(records)
            result = helper(records)
            self.assertEqual(helper(list(reversed(records))), result)
            self.assertEqual(records, original)
            snapshot = copy.deepcopy(result)
            records[0].clear()
            self.assertEqual(result, snapshot)

    def test_every_producer_field_affects_identity_and_malformed_records_fail(self) -> None:
        records = self.producers()
        original = baseline.producer_set_identity(records)
        for field in ("id", "adapter", "producer", "version"):
            changed = copy.deepcopy(records)
            changed[0][field] += "-different"
            self.assertNotEqual(baseline.producer_set_identity(changed)["sha256"], original["sha256"])
            for value in (None, 1, True, ""):
                changed[0][field] = value
                with self.subTest(field=field, value=value), self.assertRaises(baseline.BaselineError):
                    baseline.producer_set_identity(changed)
        for invalid in ([], None, {}, [records[0], records[0]], [dict(records[0], extra="no")], [{"id": "only"}]):
            with self.subTest(invalid=invalid), self.assertRaises(baseline.BaselineError):
                baseline.producer_set_identity(invalid)
        changed = copy.deepcopy(records)
        changed[1]["id"] = changed[0]["id"]
        with self.assertRaisesRegex(baseline.BaselineError, "duplicate"):
            baseline.producer_set_identity(changed)

    def test_all_tcp_fields_and_types_are_part_of_the_complete_key(self) -> None:
        projection = self.scenarios()[0]
        original = baseline.scenario_set_identity([projection])["sha256"]
        for field in projection["identity"]:
            changed = copy.deepcopy(projection)
            if field == "transport":
                changed["identity"][field] = "rtu"
                with self.assertRaises(baseline.BaselineError):
                    baseline.scenario_set_identity([changed])
            else:
                changed["identity"][field] = "mixed" if field == "operation" else changed["identity"][field] + 1
                self.assertNotEqual(baseline.scenario_set_identity([changed])["sha256"], original, field)
            for invalid in (True, "1", 1.0, None):
                changed = copy.deepcopy(projection)
                changed["identity"][field] = invalid
                with self.subTest(field=field, invalid=invalid), self.assertRaises(baseline.BaselineError):
                    baseline.scenario_set_identity([changed])
            changed = copy.deepcopy(projection)
            del changed["identity"][field]
            with self.assertRaises(baseline.BaselineError):
                baseline.scenario_set_identity([changed])

    def test_criterion_kind_producer_and_duplicate_keys_are_strict(self) -> None:
        records = self.scenarios()
        changed = copy.deepcopy(records)
        changed[1]["identity"]["benchmark_id"] = "codec/encode"
        self.assertNotEqual(baseline.scenario_set_identity(changed)["sha256"], baseline.scenario_set_identity(records)["sha256"])
        for index in (0, 1):
            for field, value in (("kind", "unsupported"), ("producer_id", "other"),
                                 ("identity", {}), ("extra", "not permitted")):
                changed = copy.deepcopy(records)
                changed[index][field] = value
                with self.subTest(index=index, field=field), self.assertRaises(baseline.BaselineError):
                    baseline.scenario_set_identity(changed)
            with self.assertRaisesRegex(baseline.BaselineError, "duplicate"):
                baseline.scenario_set_identity([records[index], copy.deepcopy(records[index])])
        for invalid in ([], None, {}, [dict(records[1], sources=[])],
                        [dict(records[1], identity={"benchmark_id": "x", 1: 0, "extra": 0})]):
            with self.subTest(invalid=invalid), self.assertRaises(baseline.BaselineError):
                baseline.scenario_set_identity(invalid)


class ArtifactIdentityTests(unittest.TestCase):
    def setUp(self) -> None:
        # Compose just the existing fixture; do not inherit and rerun its test suite.
        self.f = ControlledArtifactBindingTests("test_complete_bindings_match_independent_hashes_and_identity_read_only")
        self.addCleanup(self.f.doCleanups)
        self.f.setUp()

    def encoded(self, document: dict) -> bytes:
        return (json.dumps(
            document, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False
        ) + "\n").encode("utf-8")

    def independent_sets(self, report: dict) -> dict:
        producers = sorted(copy.deepcopy(report["producers"]), key=lambda item: item["id"].encode("utf-8"))
        scenarios = [{key: copy.deepcopy(item[key]) for key in ("kind", "producer_id", "identity")}
                     for item in report["scenarios"]]
        result = {}
        for key, name, records in (
            ("producers", "producer", producers),
            ("scenarios", "scenario", sorted(scenarios, key=self.encoded)),
        ):
            preimage = {"set_schema": {"name": f"benchmark-{name}-set", "version": 1}, key: records}
            result[f"{name}_set"] = {"preimage": preimage, "sha256": hashlib.sha256(self.encoded(preimage)).hexdigest()}
        return result

    def enable_v2(self) -> dict:
        expected = None
        for artifact in self.f.artifacts.values():
            report = json.loads((artifact.run_dir / "benchmark-report-v1.json").read_text())
            identities = self.independent_sets(report)
            if expected is not None:
                self.assertEqual(identities, expected)
            expected = identities
        assert expected is not None
        for name in ("producer", "scenario"):
            field = f"{name}_set_sha256"
            self.f.study[field] = expected[f"{name}_set"]["sha256"]
            for run in self.f.study["runs"]:
                run[field] = self.f.study[field]
            self.f.bindings[f"{name}_set_schema"] = {"name": f"benchmark-{name}-set", "version": 1}
        self.f.bindings["binding_schema"]["version"] = 2
        self.f.refresh_contract_pin()
        return expected

    def test_artifact_identity_cli_rebuilds_full_source_read_only(self) -> None:
        artifact = self.f.artifacts["evidence-run-current-a"]
        relative = artifact.run_dir.relative_to(self.f.root).as_posix()
        report = json.loads((artifact.run_dir / "benchmark-report-v1.json").read_text())
        expected = self.independent_sets(report)
        before = self.f.inventory()
        result = self.f.cli("artifact-identities", relative, ascii_stdout=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, b"")
        document = json.loads(result.stdout)
        self.assertEqual(document["identity_schema"], {"name": "benchmark-artifact-identities", "version": 1})
        for field in ("producer_set", "scenario_set"):
            self.assertEqual(document[field], expected[field])
        self.assertEqual(document["source"], {"target_sha": self.f.sha, "run_id": artifact.run_id, "mode": "bench-full"})
        self.assertEqual(result.stdout, self.encoded(document))
        self.assertEqual(self.f.inventory(), before)

    def test_manifest_v2_matches_independently_derived_sets(self) -> None:
        expected = self.enable_v2()
        before = self.f.inventory()
        result = self.f.verify()
        self.assertEqual(result["verification_schema"]["version"], 2)
        for row in result["verified_artifacts"]:
            for name in ("producer", "scenario"):
                self.assertEqual(row[f"{name}_set_sha256"], expected[f"{name}_set"]["sha256"])
        self.assertEqual(result["not_verified"], list(baseline.CONTROLLED_ARTIFACT_NOT_VERIFIED)[2:])
        self.assertEqual(result["performance_enforcement"]["state"], "not_eligible")
        self.assertEqual(self.f.inventory(), before)
        self.f.bindings["artifacts"].reverse()
        self.f.study["runs"].reverse()
        self.f.write_documents()  # Canonical pins/results must survive these permutations.
        self.assertEqual(self.f.verify(), result)

    def test_v1_manifest_and_result_bytes_are_frozen(self) -> None:
        # Characterization against the parent before implementation; all result keys
        # and qualifications are explicitly frozen, not inferred from the result.
        manifest_bytes = self.encoded(self.f.bindings)
        expected = {
            "verification_schema": {"name": "benchmark-controlled-artifact-verification", "version": 1},
            "contract": {"contract_id": self.f.contract["contract_id"], "canonical_sha256": self.f.bindings["contract_sha256"]},
            "binding_manifest": {"schema": {"name": "benchmark-controlled-artifact-bindings", "version": 1},
                                 "canonical_sha256": hashlib.sha256(manifest_bytes).hexdigest()},
            "artifact_content_schema": {"name": "benchmark-artifact-content", "version": 1},
            "verified_artifacts": self.f.expected_bindings,
            "verification_scope": "variance_run_artifact_content_and_declared_run_identity_only",
            "qualification": "integrity_only_not_authentication_or_owner_authorization",
            "performance_enforcement": {"state": "not_eligible", "reason": "artifact_binding_verification_only"},
            "not_verified": [
                "producer_set_sha256", "scenario_set_sha256", "budget_scenario_identity_sha256",
                "runner_profile_control_and_environment_equality", "statistical_method_and_variance_analysis",
                "independent_executions", "non_artifact_and_unmapped_retained_evidence",
                "expiration_and_continued_retention", "approval_authentication_and_owner_authorization",
                "baseline_acceptance", "performance_enforcement",
            ],
        }
        self.assertEqual(baseline.controlled_artifact_bindings_json_text(self.f.bindings).encode("utf-8"), manifest_bytes)
        result = self.f.cli("verify-controlled-artifacts", self.f.contract_relative, self.f.bindings_relative)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, self.encoded(expected))
        self.assertEqual(result.stderr, b"")

    def assert_failure(self, relative: str | None = None, *, before_artifacts: bool = False) -> None:
        with contextlib.ExitStack() as stack:
            if before_artifacts:
                stack.enter_context(mock.patch.object(
                    baseline, "_load_benchmark_artifact_evidence", side_effect=AssertionError("premature evidence load")
                ))
            if relative is None:
                self.f.assert_failure(before_artifacts=before_artifacts)
            else:
                before = self.f.inventory()
                with self.assertRaises(baseline.BaselineError):
                    baseline.artifact_identities(self.f.root, relative)
                result = self.f.cli("artifact-identities", relative)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, b"")
                self.assertNotIn(b"Traceback", result.stderr)
                self.assertEqual(self.f.inventory(), before)

    def refresh_content_pin(self, evidence_id: str) -> None:
        artifact = self.f.artifacts[evidence_id]
        digest = self.f.independent_content_sha256(artifact.run_dir)
        next(item for item in self.f.contract["evidence_retention"] if item["evidence_id"] == evidence_id)["sha256"] = digest
        self.f.refresh_contract_pin()

    def rewrite_stress(self, artifact: baseline.ArtifactRun, *, duration_delta: int = 0, measurement_delta: float = 0) -> None:
        # Update a retained synthetic source coherently without running a benchmark.
        summary_path = artifact.run_dir / "summary.json"
        summary = json.loads(summary_path.read_text())
        for sample in summary["stress_samples"]:
            sample["duration_secs"] += duration_delta
            sample["throughput_ops_sec"] += measurement_delta
            sample["per_client_ops_sec"] += measurement_delta
            parsed = artifact.run_dir / "stress/parsed" / f"stress-{sample['operation']}-d{sample['in_flight']}-r{sample['repetition']}.json"
            baseline.write_json(parsed, sample)
            raw = {key: value for key, value in sample.items() if key not in ("command_id", "repetition")}
            baseline.write_json(artifact.run_dir / "commands" / sample["command_id"] / "command.stdout", raw)
        repetitions = max(item["repetition"] for item in summary["stress_samples"])
        summary["stress_aggregates"] = baseline.aggregate_stress_samples(
            summary["stress_samples"], baseline.stress_scenarios(artifact.mode, repetitions)
        )
        baseline.write_json(summary_path, summary)
        baseline.write_summary_csv(artifact.run_dir / "summary.csv", summary["stress_aggregates"], summary["criterion_results"])
        report = baseline.build_benchmark_report(self.f.root, artifact.run_dir, require_artifact_checksums=False)
        for name in ("benchmark-report-v1.json", "benchmark-report-v1.md"):
            (artifact.run_dir / name).unlink()  # Replace only task-owned synthetic fixture views.
        baseline.write_report_pair(artifact.run_dir, report)
        baseline.write_checksums(self.f.root, artifact.run_dir)

    def test_pure_projection_excludes_metrics_paths_run_metadata_and_runner(self) -> None:
        artifact = self.f.artifacts["evidence-run-current-a"]
        report = baseline.build_benchmark_report(self.f.root, artifact.run_dir)
        shifted = shifted_report_fixture(report)
        criterion = next(item for item in shifted["scenarios"] if item["kind"] == "criterion_estimate")
        criterion["sources"][0]["private_estimates_json"] = criterion["sources"][0]["private_estimates_json"].replace("01-tcp_throughput", "99-other")
        shifted["source_artifact"]["provenance"]["started_utc"] = "2020-01-01T00:00:00Z"
        self.assertEqual(baseline.validate_report_document(shifted), [])
        self.assertEqual(baseline._report_identity_sets(report), baseline._report_identity_sets(shifted))
        reordered = copy.deepcopy(report)
        reordered["producers"].reverse()
        self.assertTrue(baseline.validate_report_document(reordered))  # Existing order remains strict.
        self.assertEqual(baseline.producer_set_identity(report["producers"]), baseline.producer_set_identity(reordered["producers"]))

    def test_metrics_only_changes_match_v2_after_whole_content_pin_refresh(self) -> None:
        expected = self.enable_v2()
        artifact = self.f.artifacts["evidence-run-current-b"]
        old_content = self.f.independent_content_sha256(artifact.run_dir)
        self.rewrite_stress(artifact, measurement_delta=25)
        self.assertNotEqual(self.f.independent_content_sha256(artifact.run_dir), old_content)
        self.refresh_content_pin("evidence-run-current-b")
        actual = baseline.artifact_identities(self.f.root, artifact.run_dir.relative_to(self.f.root).as_posix())
        for field in ("producer_set", "scenario_set"):
            self.assertEqual(actual[field], expected[field])
        self.assertEqual(self.f.verify()["verification_schema"]["version"], 2)

    def test_real_workload_change_fails_second_binding_but_v1_stays_structural(self) -> None:
        expected = self.enable_v2()
        artifact = self.f.artifacts["evidence-run-current-b"]
        self.rewrite_stress(artifact, duration_delta=1)
        self.refresh_content_pin("evidence-run-current-b")
        actual = baseline.artifact_identities(self.f.root, artifact.run_dir.relative_to(self.f.root).as_posix())
        self.assertEqual(actual["producer_set"], expected["producer_set"])
        self.assertNotEqual(actual["scenario_set"]["sha256"], expected["scenario_set"]["sha256"])
        with self.assertRaisesRegex(baseline.BaselineError, "scenario-set"):
            self.f.verify()
        self.assert_failure()
        self.f.bindings["binding_schema"]["version"] = 1
        del self.f.bindings["producer_set_schema"], self.f.bindings["scenario_set_schema"]
        self.f.write_documents()
        self.f.assert_verification_scope(self.f.verify())

    def test_actual_producer_record_mismatch_and_unsupported_source_fail_closed(self) -> None:
        expected = self.enable_v2()
        artifact = self.f.artifacts["evidence-run-current-b"]
        report = json.loads((artifact.run_dir / "benchmark-report-v1.json").read_text())
        report["producers"][0]["adapter"] += " changed"
        different = self.independent_sets(report)["producer_set"]["sha256"]
        self.assertNotEqual(different, expected["producer_set"]["sha256"])
        self.f.study["producer_set_sha256"] = different
        for run in self.f.study["runs"]:
            run["producer_set_sha256"] = different
        self.f.refresh_contract_pin()
        with self.assertRaisesRegex(baseline.BaselineError, "producer-set"):
            self.f.verify()
        self.assert_failure()
        provenance_path = artifact.run_dir / "provenance.json"
        provenance = json.loads(provenance_path.read_text())
        provenance["harness_version"] = "unsupported"
        baseline.write_json(provenance_path, provenance)
        baseline.write_checksums(self.f.root, artifact.run_dir)
        self.assert_failure(artifact.run_dir.relative_to(self.f.root).as_posix())

    def test_v2_manifest_scheme_pin_coverage_and_path_errors_precede_artifact_work(self) -> None:
        self.enable_v2()
        original = copy.deepcopy(self.f.bindings)
        cases = []
        for field in ("producer_set_schema", "scenario_set_schema"):
            missing = copy.deepcopy(original)
            del missing[field]
            cases.append(missing)
            for key, value in (("version", True), ("version", 2), ("name", "unsupported"), ("extra", "not permitted")):
                changed = copy.deepcopy(original)
                changed[field][key] = value
                cases.append(changed)
        for version in (True, 1, 3):
            changed = copy.deepcopy(original)
            changed["binding_schema"]["version"] = version
            cases.append(changed)
        stale = copy.deepcopy(original)
        stale["contract_sha256"] = "f" * 64
        missing_run = copy.deepcopy(original)
        missing_run["artifacts"].pop()
        invalid_path = copy.deepcopy(original)
        invalid_path["artifacts"][1]["run_dir"] = "../outside"
        cases.extend((stale, missing_run, invalid_path))
        for position, document in enumerate(cases):
            with self.subTest(case=position):
                self.f.bindings = document
                self.f.write_documents()
                self.assert_failure(before_artifacts=True)

    def test_v2_does_not_relax_content_or_source_identity_checks(self) -> None:
        expected = self.enable_v2()
        a, b = self.f.bindings["artifacts"]
        a["run_dir"], b["run_dir"] = b["run_dir"], a["run_dir"]
        self.f.write_documents()
        with self.assertRaisesRegex(baseline.BaselineError, "run identity"):
            self.f.verify()  # Both sets still match; source run identity does not.
        self.assert_failure()
        a["run_dir"], b["run_dir"] = b["run_dir"], a["run_dir"]
        self.f.write_documents()
        artifact = self.f.artifacts["evidence-run-current-b"]
        next(artifact.run_dir.glob("commands/*/command.stderr")).write_bytes(b"changed raw bytes")
        baseline.write_checksums(self.f.root, artifact.run_dir)
        actual = baseline.artifact_identities(self.f.root, b["run_dir"])
        for field in ("producer_set", "scenario_set"):
            self.assertEqual(actual[field], expected[field])
        with self.assertRaisesRegex(baseline.BaselineError, "content digest"):
            self.f.verify()
        self.assert_failure()

    def test_smoke_derivation_succeeds_but_v2_still_requires_full(self) -> None:
        self.enable_v2()
        smoke = self.f.make_artifact("identity-smoke", mode="bench-smoke")
        relative = smoke.run_dir.relative_to(self.f.root).as_posix()
        # A real Unicode Criterion identity must be emitted as UTF-8 even when
        # stdout's text encoding is ASCII. Leave the copied report stale deliberately.
        summary_path = smoke.run_dir / "summary.json"
        summary = json.loads(summary_path.read_text())
        criterion = summary["criterion_results"][0]
        new_source = smoke.run_dir / "criterion/raw/01-tcp_throughput/épreuve/new/estimates.json"
        new_source.parent.mkdir(parents=True)
        shutil.copyfile(self.f.root / criterion["source"], new_source)
        criterion["source"] = new_source.relative_to(self.f.root).as_posix()
        criterion["benchmark_id"] = "épreuve"
        baseline.write_json(summary_path, summary)
        baseline.write_json(smoke.run_dir / "criterion/parsed-estimates.json", summary["criterion_results"])
        baseline.write_checksums(self.f.root, smoke.run_dir)
        result = self.f.cli("artifact-identities", relative, ascii_stdout=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["source"]["mode"], "bench-smoke")
        self.assertIn('"benchmark_id":"épreuve"'.encode("utf-8"), result.stdout)
        self.f.study["runs"][1]["run_id"] = smoke.run_id
        self.f.bindings["artifacts"][1]["run_dir"] = relative
        next(item for item in self.f.contract["evidence_retention"] if item["evidence_id"] == "evidence-run-current-b")["sha256"] = self.f.independent_content_sha256(smoke.run_dir)
        self.f.refresh_contract_pin()
        with self.assertRaisesRegex(baseline.BaselineError, "bench-full"):
            self.f.verify()
        self.assert_failure()

    def test_duplicate_criterion_keys_with_distinct_sources_do_not_weaken_v1(self) -> None:
        smoke = self.f.make_artifact("duplicate-smoke", mode="bench-smoke")
        summary_path = smoke.run_dir / "summary.json"
        summary = json.loads(summary_path.read_text())
        duplicate = copy.deepcopy(summary["criterion_results"][0])
        new_source = smoke.run_dir / "criterion/raw/02-other" / duplicate["benchmark_id"] / "new/estimates.json"
        new_source.parent.mkdir(parents=True)
        shutil.copyfile(self.f.root / duplicate["source"], new_source)
        duplicate["source"] = new_source.relative_to(self.f.root).as_posix()
        summary["criterion_results"].append(duplicate)
        baseline.write_json(summary_path, summary)
        baseline.write_json(smoke.run_dir / "criterion/parsed-estimates.json", summary["criterion_results"])
        baseline.write_checksums(self.f.root, smoke.run_dir)
        relative = smoke.run_dir.relative_to(self.f.root).as_posix()
        baseline.fingerprint_artifact(self.f.root, relative)  # Preserve old admission behavior.
        self.assert_failure(relative)  # New comparison-based identities reject the duplicate.

    def test_derivation_rejects_incomplete_dirty_malformed_and_copy_only_sources(self) -> None:
        artifact = self.f.artifacts["evidence-run-current-b"]
        relative = artifact.run_dir.relative_to(self.f.root).as_posix()
        summary_path = artifact.run_dir / "summary.json"
        provenance_path = artifact.run_dir / "provenance.json"
        summary_bytes, provenance_bytes = summary_path.read_bytes(), provenance_path.read_bytes()
        for case in ("malformed", "dirty", "incomplete"):
            with self.subTest(case=case):
                summary_path.write_bytes(summary_bytes)
                provenance_path.write_bytes(provenance_bytes)
                if case == "malformed":
                    summary_path.write_bytes(b"\xff")
                elif case == "dirty":
                    provenance = json.loads(provenance_bytes)
                    provenance.update(dirty=True, dirty_override=True)
                    baseline.write_json(provenance_path, provenance)
                else:
                    summary = json.loads(summary_bytes)
                    summary.update(stress_samples=[], stress_aggregates=[])
                    baseline.write_json(summary_path, summary)
                baseline.write_checksums(self.f.root, artifact.run_dir)
                self.assert_failure(relative)  # The copied complete report was never changed.
        self.assert_failure(relative + "/benchmark-report-v1.json")

    def test_shared_loader_guards_unsafe_trees_before_any_payload_read(self) -> None:
        artifact = self.f.artifacts["evidence-run-current-a"]
        relative = artifact.run_dir.relative_to(self.f.root).as_posix()
        link = artifact.run_dir / "outside-link"
        try:
            link.symlink_to(self.f.script)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"symlinks unavailable: {error}")
        with mock.patch.object(Path, "open", side_effect=AssertionError("unsafe payload read")):
            with self.assertRaises(baseline.BaselineError):
                baseline.artifact_identities(self.f.root, relative)
        self.assert_failure(relative)
        link.unlink()
        if hasattr(os, "mkfifo"):
            os.mkfifo(link)
            with mock.patch.object(Path, "open", side_effect=AssertionError("FIFO read")):
                with self.assertRaises(baseline.BaselineError):
                    baseline.artifact_identities(self.f.root, relative)
            self.assert_failure(relative)

    def test_missing_local_objects_fail_for_derivation_and_v2(self) -> None:
        self.enable_v2()
        (self.f.root / ".git/objects" / self.f.sha[:2] / self.f.sha[2:]).unlink()
        self.assert_failure(self.f.bindings["artifacts"][0]["run_dir"])
        self.assert_failure()

    def test_v2_single_validation_per_artifact_and_release_between_runs(self) -> None:
        self.enable_v2()
        original_loader = baseline._load_benchmark_artifact_evidence
        previous = []
        visited = []

        class Tracked(dict):
            pass

        def load(root, path):
            for reference in previous:
                self.assertIsNone(reference(), "full report/inventory retained across runs")
            fingerprint, report = original_loader(root, path)
            fingerprint, report = Tracked(fingerprint), Tracked(report)
            previous[:] = [weakref.ref(fingerprint), weakref.ref(report)]
            visited.append(path)
            return fingerprint, report

        with mock.patch.object(baseline, "_load_benchmark_artifact_evidence", load), \
                mock.patch.object(baseline, "build_benchmark_report", wraps=baseline.build_benchmark_report) as rebuild, \
                mock.patch.object(baseline, "fingerprint_artifact", side_effect=AssertionError("duplicate public fingerprint pass")):
            self.f.verify()
        self.assertEqual(rebuild.call_count, 2)
        self.assertEqual(visited, [item["run_dir"] for item in self.f.bindings["artifacts"]])
        self.assertTrue(all(reference() is None for reference in previous))

    def test_v2_output_and_new_commands_are_read_only_local_git_only(self) -> None:
        self.enable_v2()
        actual_run, actual_open = subprocess.run, Path.open
        calls = []

        def local_git(argv, **kwargs):
            self.assertEqual(tuple(argv[:5]), ("git", "--no-lazy-fetch", "--no-replace-objects", "cat-file", "blob"))
            self.assertIn(argv[5], (f"{self.f.sha}:Cargo.lock", f"{self.f.sha}:benchmarks/Cargo.toml"))
            calls.append(argv[5])
            return actual_run(argv, **kwargs)

        def read_only_open(path, mode="r", *args, **kwargs):
            self.assertIn(mode, ("r", "rb"))
            self.assertTrue(path in (self.f.contract_path, self.f.bindings_path) or any(
                path.is_relative_to(artifact.run_dir) for artifact in self.f.artifacts.values()
            ))
            return actual_open(path, mode, *args, **kwargs)

        before = self.f.inventory()
        output, stderr = io.BytesIO(), io.StringIO()
        stdout = io.TextIOWrapper(output, encoding="utf-8")
        with contextlib.ExitStack() as stack:
            for name in ("run_mode", "run_benchmarks", "bootstrap_repository", "collect_environment", "utc_now",
                         "load_policy_file", "controlled_evaluate_artifacts", "write_json", "write_checksums"):
                stack.enter_context(mock.patch.object(baseline, name, side_effect=AssertionError(name)))
            for name in ("socket.create_connection", "urllib.request.urlopen"):
                stack.enter_context(mock.patch(name, side_effect=AssertionError(name)))
            stack.enter_context(mock.patch.object(baseline.subprocess, "run", local_git))
            stack.enter_context(mock.patch.object(Path, "open", read_only_open))
            stack.enter_context(mock.patch.object(baseline, "__file__", str(self.f.script)))
            stack.enter_context(contextlib.redirect_stdout(stdout))
            stack.enter_context(contextlib.redirect_stderr(stderr))
            identities = baseline.artifact_identities(self.f.root, self.f.bindings["artifacts"][0]["run_dir"])
            self.assertEqual(baseline.main(["verify-controlled-artifacts", self.f.contract_relative, self.f.bindings_relative]), 0)
        result = json.loads(output.getvalue())
        self.assertEqual(len(calls), 6)
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(result["not_verified"], list(baseline.CONTROLLED_ARTIFACT_NOT_VERIFIED)[2:])
        self.assertEqual(result["qualification"], "integrity_only_not_authentication_producer_execution_attestation_or_owner_authorization")
        self.assertEqual(result["verification_scope"], "variance_run_artifact_content_run_identity_and_producer_scenario_sets_only")
        self.assertEqual(result["performance_enforcement"]["state"], "not_eligible")
        for name in ("producer", "scenario"):
            self.assertEqual(result[f"{name}_set_schema"], identities[f"{name}_set"]["preimage"]["set_schema"])
        for forbidden in (b'"preimage"', b'"files"', b'"approved"'):
            self.assertNotIn(forbidden, output.getvalue())
        self.assertEqual(self.f.inventory(), before)
        cli = self.f.cli("verify-controlled-artifacts", self.f.contract_relative, self.f.bindings_relative, ascii_stdout=True)
        self.assertEqual(cli.returncode, 0, cli.stderr)
        self.assertEqual(cli.stdout, output.getvalue())

    def test_new_help_and_usage_errors(self) -> None:
        for args in (("--help",), ("artifact-identities", "--help"), ("verify-controlled-artifacts", "--help")):
            result = self.f.cli(*args)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stderr, b"")
        for args in (("artifact-identities",), ("artifact-identities", "missing", "--latest")):
            result = self.f.cli(*args)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertEqual(result.stdout, b"")


if __name__ == "__main__":
    unittest.main(verbosity=2)
