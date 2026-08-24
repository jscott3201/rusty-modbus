#!/usr/bin/env python3
"""Focused tests for the reproducible baseline harness."""

from __future__ import annotations

import copy
import contextlib
import io
import json
import math
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


class BaselineHarnessTests(unittest.TestCase):
    def make_run(
        self,
        root: Path,
        *,
        run_id: str = "test-run",
        dirty: bool = False,
        allow_dirty: bool = False,
    ) -> baseline.ArtifactRun:
        run = baseline.ArtifactRun(
            repo_root=root,
            output_root=root / "bench-output",
            target_sha=SHA,
            run_id=run_id,
            mode="bench-smoke",
            runner_label="unit-test",
            dirty=dirty,
            allow_dirty=allow_dirty,
        )
        run.create()
        return run

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
            "bench-smoke", ROOT, ["codec", "tcp_throughput"], ROOT / "artifact"
        )
        self.assertEqual([spec.label for spec in specs], ["criterion-tcp_throughput"])
        self.assertTrue(all(spec.argv[-2:] == ("--quick", "--noplot") for spec in specs))
        self.assertIn("tcp_pipelined", specs[0].argv)

        full_specs = baseline.benchmark_criterion_specs(
            "bench-full",
            ROOT,
            [
                "codec",
                "rtu_tcp_latency",
                "server_handler",
                "tcp_latency",
                "tcp_throughput",
                "tls_latency",
            ],
            ROOT / "artifact",
        )
        self.assertEqual(
            [spec.label for spec in full_specs],
            [
                "criterion-tcp_latency",
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
