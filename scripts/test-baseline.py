#!/usr/bin/env python3
"""Focused tests for the reproducible baseline harness."""

from __future__ import annotations

import copy
import contextlib
import io
import json
import math
import shutil
import subprocess
import sys
import tempfile
import unittest
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


def populate_benchmark_evidence(run: baseline.ArtifactRun) -> None:
    scenarios = baseline.stress_scenarios("bench-smoke", 1)
    parsed_dir = run.run_dir / "stress" / "parsed"
    parsed_dir.mkdir(parents=True)
    for index, scenario in enumerate(scenarios, 1):
        sample = stress_fixture(
            operation=scenario["operation"],
            in_flight=scenario["in_flight"],
            warmup_secs=1,
            throughput_ops_sec=float(100 + index),
            per_client_ops_sec=float(100 + index),
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
    criterion_home = run.run_dir / "criterion" / "raw" / "01-tcp-throughput"
    estimate = criterion_home / "tcp_pipelined" / "new" / "estimates.json"
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
    run.criterion_results = baseline.parse_criterion_estimates(
        criterion_home, run.repo_root
    )
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


class BaselineHarnessTests(unittest.TestCase):
    def make_run(
        self,
        root: Path,
        *,
        run_id: str = "test-run",
        dirty: bool = False,
        allow_dirty: bool = False,
        target_sha: str = SHA,
    ) -> baseline.ArtifactRun:
        run = baseline.ArtifactRun(
            repo_root=root,
            output_root=root / "bench-output",
            target_sha=target_sha,
            run_id=run_id,
            mode="bench-smoke",
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
    ) -> baseline.ArtifactRun:
        fixture_sha = initialize_git_lock_fixture(
            root,
            criterion_versions=criterion_versions,
            include_lock=include_lock,
        )
        return self.make_run(
            root,
            run_id=run_id,
            target_sha=target_sha or fixture_sha,
        )

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
            root = Path(directory)
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
            root = Path(directory)
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
            parent = Path(directory)
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
