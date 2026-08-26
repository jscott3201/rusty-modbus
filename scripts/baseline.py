#!/usr/bin/env python3
"""Create and validate reproducible correctness and benchmark evidence."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import platform
import re
import statistics
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path, PurePosixPath
from typing import Any, Sequence

SCHEMA_VERSION = 1
HARNESS_VERSION = "1"
HARNESS_PATH = "scripts/baseline.py"
REPORT_SCHEMA_NAME = "benchmark-report"
REPORT_SCHEMA_VERSION = 1
REPORT_JSON_NAME = "benchmark-report-v1.json"
REPORT_MARKDOWN_NAME = "benchmark-report-v1.md"
COMPARISON_SCHEMA_NAME = "benchmark-comparison"
COMPARISON_SCHEMA_VERSION = 1
STRESS_PRODUCER_ID = "rusty-modbus-stress-json-v1"
CRITERION_PRODUCER_ID = "criterion-0.5.1-private-estimates-layout"
SUPPORTED_CRITERION_VERSION = "0.5.1"
CRITERION_ADAPTER = "new/estimates.json private layout"
DEFAULT_OUTPUT_ROOT = "bench-output"
FULL_SHA = re.compile(r"[0-9a-f]{40}\Z")
RUN_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}\Z")
RUNNER_LABEL = re.compile(r"[^\x00-\x1f\x7f]{1,128}\Z")
COMMAND_ID = re.compile(r"[0-9]{3}-[a-z0-9-]+\Z")
MODES = ("correctness", "bench-smoke", "bench-full")
BENCHMARK_MODES = ("bench-smoke", "bench-full")
REPORT_EVIDENCE = {
    "artifact_validity": "valid",
    "budget_decision": "not_evaluated",
    "classification": "observational_only",
    "performance_comparability": "not_proven",
    "runner_isolation": "not_proven",
    "statistical_significance": "not_evaluated",
}
COMPARISON_EVIDENCE = {
    "budget_decision": "not_evaluated",
    "classification": "observational_only",
    "performance_comparability": "not_proven",
    "runner_isolation": "not_proven",
    "statistical_significance": "not_evaluated",
}
GITHUB_ENV_ALLOWLIST = (
    "GITHUB_ACTIONS",
    "GITHUB_JOB",
    "GITHUB_REF",
    "GITHUB_REF_NAME",
    "GITHUB_RUN_ATTEMPT",
    "GITHUB_RUN_ID",
    "GITHUB_SHA",
    "GITHUB_WORKFLOW",
    "RUNNER_ARCH",
    "RUNNER_NAME",
    "RUNNER_OS",
)
CSV_COLUMNS = (
    "record_type",
    "transport",
    "operation",
    "in_flight",
    "clients",
    "registers",
    "repetitions",
    "throughput_count",
    "throughput_min",
    "throughput_median",
    "throughput_mean",
    "throughput_max",
    "throughput_sample_stddev",
    "throughput_cv",
    "p99_count",
    "p99_min_ms",
    "p99_median_ms",
    "p99_mean_ms",
    "p99_max_ms",
    "p99_sample_stddev_ms",
    "p99_cv",
    "total_errors",
    "retry_attempts",
    "criterion_id",
    "criterion_mean_lower_ns",
    "criterion_mean_point_ns",
    "criterion_mean_upper_ns",
)
STRESS_NUMERIC_FIELDS = (
    "throughput_ops_sec",
    "per_client_ops_sec",
    "error_rate",
)
STRESS_INTEGER_FIELDS = (
    "schema_version",
    "clients",
    "in_flight",
    "duration_secs",
    "warmup_secs",
    "registers",
    "total_ops",
    "errors",
    "retry_attempts",
)
LATENCY_FIELDS = ("p50", "p95", "p99", "p999", "min", "max")
MEMORY_FIELDS = ("rss_before_mb", "rss_after_mb", "delta_mb")


class BaselineError(Exception):
    """Raised when evidence cannot be produced or validated."""


class CommandFailure(BaselineError):
    """Raised after a failed command has been recorded."""


@dataclass(frozen=True)
class CommandSpec:
    label: str
    argv: tuple[str, ...]
    cwd: Path
    env: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True)
class BootstrapResult:
    argv: tuple[str, ...]
    cwd: Path
    stdout: bytes
    stderr: bytes
    exit_code: int | None
    started_utc: str
    ended_utc: str
    duration_seconds: float
    error: str | None = None


@dataclass(frozen=True)
class CommandResult:
    command_id: str
    argv: tuple[str, ...]
    cwd: Path
    stdout: bytes
    stderr: bytes
    exit_code: int | None
    status: str


def utc_now() -> str:
    return datetime.now(UTC).isoformat(timespec="microseconds").replace("+00:00", "Z")


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
        newline="\n",
    )


def validate_full_sha(value: str) -> str:
    if not FULL_SHA.fullmatch(value):
        raise BaselineError(f"target SHA must be 40 lowercase hexadecimal characters: {value!r}")
    return value


def validate_run_id(value: str) -> str:
    if value in {".", ".."} or not RUN_ID.fullmatch(value):
        raise BaselineError(
            "run ID must be 1-64 ASCII letters, digits, dots, underscores, or hyphens; "
            "it must start with a letter or digit"
        )
    return value


def validate_runner_label(value: str) -> str:
    if not RUNNER_LABEL.fullmatch(value):
        raise BaselineError("runner label must be 1-128 characters without control characters")
    return value


def enforce_clean_worktree(dirty: bool, allow_dirty: bool) -> None:
    if dirty and not allow_dirty:
        raise BaselineError(
            "non-ignored worktree is dirty; use --allow-dirty only for invalid diagnostics"
        )


def resolve_output_root(repo_root: Path, value: str) -> Path:
    raw = Path(value)
    if not raw.is_absolute():
        pure = PurePosixPath(value.replace(os.sep, "/"))
        if ".." in pure.parts or pure.is_absolute():
            raise BaselineError("relative output root must not contain path traversal")
        raw = repo_root / raw
    resolved = raw.resolve()
    try:
        relative = resolved.relative_to(repo_root.resolve())
    except ValueError as error:
        raise BaselineError("output root must be inside the repository") from error
    if not relative.parts:
        raise BaselineError("output root must not be the repository root")
    return resolved


def default_run_id() -> str:
    return datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ") + f"-{os.getpid()}"


def _run_bootstrap(argv: Sequence[str], cwd: Path) -> BootstrapResult:
    started = utc_now()
    start = time.monotonic()
    try:
        completed = subprocess.run(
            list(argv), cwd=cwd, capture_output=True, check=False, shell=False
        )
        exit_code: int | None = completed.returncode
        stdout = completed.stdout
        stderr = completed.stderr
        error = None
    except OSError as caught:
        exit_code = None
        stdout = b""
        stderr = str(caught).encode("utf-8", errors="replace")
        error = f"{type(caught).__name__}: {caught}"
    return BootstrapResult(
        argv=tuple(argv),
        cwd=cwd.resolve(),
        stdout=stdout,
        stderr=stderr,
        exit_code=exit_code,
        started_utc=started,
        ended_utc=utc_now(),
        duration_seconds=time.monotonic() - start,
        error=error,
    )


def bootstrap_repository(repo_root: Path) -> tuple[dict[str, Any], list[BootstrapResult]]:
    commands = [
        _run_bootstrap(("git", "rev-parse", "HEAD"), repo_root),
        _run_bootstrap(
            ("git", "status", "--porcelain", "--untracked-files=all"), repo_root
        ),
        _run_bootstrap(("git", "symbolic-ref", "--short", "-q", "HEAD"), repo_root),
    ]
    if commands[0].exit_code != 0:
        raise BaselineError("cannot resolve repository HEAD")
    if commands[1].exit_code != 0:
        raise BaselineError("cannot inspect non-ignored worktree state")
    target_sha = validate_full_sha(commands[0].stdout.decode().strip())
    dirty = bool(commands[1].stdout.strip())
    branch = commands[2].stdout.decode("utf-8", errors="replace").strip() or None
    return (
        {
            "target_sha": target_sha,
            "branch": branch,
            "detached": branch is None,
            "dirty": dirty,
        },
        commands,
    )


def _slug(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", value.casefold()).strip("-")
    return slug[:48] or "command"


class ArtifactRun:
    def __init__(
        self,
        *,
        repo_root: Path,
        output_root: Path,
        target_sha: str,
        run_id: str,
        mode: str,
        runner_label: str,
        dirty: bool,
        allow_dirty: bool,
    ) -> None:
        self.repo_root = repo_root.resolve()
        self.output_root = output_root.resolve()
        self.target_sha = validate_full_sha(target_sha)
        self.run_id = validate_run_id(run_id)
        self.mode = mode
        self.runner_label = validate_runner_label(runner_label)
        self.dirty = dirty
        self.allow_dirty = allow_dirty
        self.run_dir = (
            self.output_root / f"baseline-v{SCHEMA_VERSION}" / self.target_sha / self.run_id
        )
        self.started_utc = utc_now()
        self.command_count = 0
        self.command_records: list[dict[str, Any]] = []
        self.errors: list[str] = []
        self.stress_samples: list[dict[str, Any]] = []
        self.stress_aggregates: list[dict[str, Any]] = []
        self.criterion_results: list[dict[str, Any]] = []
        self.environment: dict[str, Any] = {}
        self.provenance: dict[str, Any] = {}
        self.finalized = False

    def create(self) -> None:
        self.run_dir.parent.mkdir(parents=True, exist_ok=True)
        try:
            self.run_dir.mkdir()
        except FileExistsError as error:
            raise BaselineError(f"run directory already exists: {self.run_dir}") from error
        (self.run_dir / "commands").mkdir()
        self.provenance = {
            "schema_version": SCHEMA_VERSION,
            "harness_version": HARNESS_VERSION,
            "harness_path": HARNESS_PATH,
            "target_sha": self.target_sha,
            "mode": self.mode,
            "run_id": self.run_id,
            "started_utc": self.started_utc,
            "ended_utc": None,
            "dirty": self.dirty,
            "dirty_override": self.allow_dirty,
            "baseline_eligible": not self.dirty,
        }
        write_json(self.run_dir / "provenance.json", self.provenance)
        write_json(
            self.run_dir / "environment.json",
            {"schema_version": SCHEMA_VERSION, "collection_status": "pending"},
        )

    def _record_result(
        self,
        *,
        label: str,
        argv: Sequence[str],
        cwd: Path,
        stdout: bytes,
        stderr: bytes,
        exit_code: int | None,
        started_utc: str,
        ended_utc: str,
        duration_seconds: float,
        env_overrides: dict[str, str] | None,
        error: str | None,
    ) -> CommandResult:
        self.command_count += 1
        command_id = f"{self.command_count:03d}-{_slug(label)}"
        command_dir = self.run_dir / "commands" / command_id
        command_dir.mkdir()
        stdout_path = command_dir / "command.stdout"
        stderr_path = command_dir / "command.stderr"
        stdout_path.write_bytes(stdout)
        stderr_path.write_bytes(stderr)
        status = "passed" if exit_code == 0 and error is None else "failed"
        record = {
            "schema_version": SCHEMA_VERSION,
            "command_id": command_id,
            "label": label,
            "argv": list(argv),
            "cwd": str(cwd.resolve()),
            "started_utc": started_utc,
            "ended_utc": ended_utc,
            "duration_seconds": duration_seconds,
            "exit_code": exit_code,
            "status": status,
            "error": error,
            "env_overrides": dict(sorted((env_overrides or {}).items())),
            "stdout_path": self._repo_relative(stdout_path),
            "stderr_path": self._repo_relative(stderr_path),
        }
        write_json(command_dir / "command.json", record)
        self.command_records.append(record)
        return CommandResult(
            command_id=command_id,
            argv=tuple(argv),
            cwd=cwd.resolve(),
            stdout=stdout,
            stderr=stderr,
            exit_code=exit_code,
            status=status,
        )

    def record_bootstrap(self, label: str, result: BootstrapResult) -> CommandResult:
        return self._record_result(
            label=label,
            argv=result.argv,
            cwd=result.cwd,
            stdout=result.stdout,
            stderr=result.stderr,
            exit_code=result.exit_code,
            started_utc=result.started_utc,
            ended_utc=result.ended_utc,
            duration_seconds=result.duration_seconds,
            env_overrides=None,
            error=result.error,
        )

    def run_command(
        self,
        spec: CommandSpec,
        *,
        required: bool = True,
        timeout: float | None = None,
    ) -> CommandResult:
        started = utc_now()
        start = time.monotonic()
        overrides = dict(spec.env)
        command_env = os.environ.copy()
        command_env.update(overrides)
        error: str | None = None
        try:
            completed = subprocess.run(
                list(spec.argv),
                cwd=spec.cwd,
                env=command_env,
                capture_output=True,
                check=False,
                shell=False,
                timeout=timeout,
            )
            stdout = completed.stdout
            stderr = completed.stderr
            exit_code: int | None = completed.returncode
        except subprocess.TimeoutExpired as caught:
            stdout = caught.stdout or b""
            stderr = caught.stderr or b""
            exit_code = None
            error = f"command timed out after {timeout} seconds"
        except OSError as caught:
            stdout = b""
            stderr = str(caught).encode("utf-8", errors="replace")
            exit_code = None
            error = f"{type(caught).__name__}: {caught}"
        result = self._record_result(
            label=spec.label,
            argv=spec.argv,
            cwd=spec.cwd,
            stdout=stdout,
            stderr=stderr,
            exit_code=exit_code,
            started_utc=started,
            ended_utc=utc_now(),
            duration_seconds=time.monotonic() - start,
            env_overrides=overrides,
            error=error,
        )
        if required and result.status != "passed":
            raise CommandFailure(
                f"command {result.command_id} failed with exit code {result.exit_code}"
            )
        return result

    def add_error(self, error: BaseException | str) -> None:
        message = str(error)
        if message not in self.errors:
            self.errors.append(message)

    def _repo_relative(self, path: Path) -> str:
        try:
            return path.resolve().relative_to(self.repo_root).as_posix()
        except ValueError as error:
            raise BaselineError(f"artifact path is outside repository: {path}") from error

    def finalize(self) -> None:
        if self.finalized:
            return
        report_error: BaselineError | None = None
        self.provenance["ended_utc"] = utc_now()
        self.provenance["baseline_eligible"] = not self.dirty and not self.errors
        write_json(self.run_dir / "provenance.json", self.provenance)
        if not self.environment:
            self.environment = {
                "schema_version": SCHEMA_VERSION,
                "collection_status": "incomplete",
            }
        write_json(self.run_dir / "environment.json", self.environment)
        if self.errors:
            status = "failed"
        elif self.dirty:
            status = "invalid"
        else:
            status = "passed"
        summary = {
            "schema_version": SCHEMA_VERSION,
            "mode": self.mode,
            "target_sha": self.target_sha,
            "run_id": self.run_id,
            "status": status,
            "baseline_valid": not self.dirty and not self.errors,
            "invalid_reasons": (["dirty non-ignored worktree"] if self.dirty else [])
            + self.errors,
            "command_count": len(self.command_records),
            "commands": [record["command_id"] for record in self.command_records],
            "stress_samples": self.stress_samples,
            "stress_aggregates": self.stress_aggregates,
            "criterion_results": self.criterion_results,
        }
        write_json(self.run_dir / "summary.json", summary)
        write_summary_csv(
            self.run_dir / "summary.csv", self.stress_aggregates, self.criterion_results
        )
        if (
            status == "passed"
            and self.mode in BENCHMARK_MODES
            and self.stress_samples
            and self.criterion_results
        ):
            try:
                report = build_benchmark_report(
                    self.repo_root, self.run_dir, require_artifact_checksums=False
                )
                write_report_pair(self.run_dir, report)
            except (BaselineError, OSError) as error:
                report_error = BaselineError(f"benchmark report generation failed: {error}")
                self.add_error(report_error)
                self.provenance["baseline_eligible"] = False
                write_json(self.run_dir / "provenance.json", self.provenance)
                summary["status"] = "failed"
                summary["baseline_valid"] = False
                summary["invalid_reasons"] = self.errors
                write_json(self.run_dir / "summary.json", summary)
        write_checksums(self.repo_root, self.run_dir)
        self.finalized = True
        if report_error is not None:
            raise report_error


def allowlisted_runner_environment(environ: dict[str, str] | None = None) -> dict[str, str]:
    source = environ if environ is not None else os.environ
    return {key: source[key] for key in GITHUB_ENV_ALLOWLIST if key in source}


def cpu_metadata() -> dict[str, Any]:
    model: str | None = platform.processor() or None
    source: str | None = "platform.processor" if model else None
    if platform.system() == "Linux":
        try:
            for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines():
                if line.casefold().startswith("model name"):
                    model = line.split(":", 1)[1].strip()
                    source = "/proc/cpuinfo"
                    break
        except OSError:
            pass
    return {
        "model": model,
        "model_source": source,
        "logical_count": os.cpu_count(),
    }


def power_metadata() -> dict[str, Any]:
    governor = Path("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
    try:
        value = governor.read_text(encoding="utf-8").strip()
    except OSError:
        return {"availability": "unavailable", "value": None, "source": None}
    return {"availability": "available", "value": value, "source": str(governor)}


def parse_rustc_verbose(text: str) -> dict[str, str | None]:
    values: dict[str, str | None] = {"version": None, "host": None}
    lines = text.splitlines()
    if lines:
        values["version"] = lines[0]
    for line in lines:
        if line.startswith("host: "):
            values["host"] = line.removeprefix("host: ")
    return values


def summarize_cargo_metadata(payload: bytes) -> tuple[dict[str, Any], list[str], Path]:
    try:
        data = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise BaselineError(f"cargo metadata output is malformed: {error}") from error
    packages = data.get("packages")
    if not isinstance(packages, list):
        raise BaselineError("cargo metadata packages must be a list")
    benchmark_targets: list[str] = []
    package_summary: list[dict[str, Any]] = []
    for package in packages:
        targets = package.get("targets", [])
        target_summary = []
        for target in targets:
            kinds = target.get("kind", [])
            name = target.get("name")
            if package.get("name") == "rusty-modbus-benchmarks" and "bench" in kinds:
                benchmark_targets.append(name)
            target_summary.append({"name": name, "kind": kinds})
        package_summary.append(
            {
                "name": package.get("name"),
                "version": package.get("version"),
                "targets": sorted(target_summary, key=lambda item: str(item["name"])),
            }
        )
    target_directory = Path(data.get("target_directory", ""))
    if not target_directory.is_absolute():
        raise BaselineError("cargo metadata target_directory must be absolute")
    summary = {
        "workspace_root": data.get("workspace_root"),
        "target_directory": str(target_directory),
        "workspace_member_count": len(data.get("workspace_members", [])),
        "packages": sorted(package_summary, key=lambda item: str(item["name"])),
    }
    return summary, sorted(benchmark_targets), target_directory


def collect_environment(run: ArtifactRun) -> tuple[list[str], Path]:
    rustc = run.run_command(
        CommandSpec("rustc-version", ("rustc", "-Vv"), run.repo_root)
    )
    cargo = run.run_command(CommandSpec("cargo-version", ("cargo", "-V"), run.repo_root))
    metadata = run.run_command(
        CommandSpec(
            "cargo-metadata",
            ("cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"),
            run.repo_root,
        )
    )
    metadata_summary, bench_targets, target_directory = summarize_cargo_metadata(
        metadata.stdout
    )
    cpu = cpu_metadata()
    power = power_metadata()
    if platform.system() == "Darwin":
        cpu_probe = run.run_command(
            CommandSpec(
                "cpu-model", ("sysctl", "-n", "machdep.cpu.brand_string"), run.repo_root
            ),
            required=False,
        )
        cpu_value = cpu_probe.stdout.decode("utf-8", errors="replace").strip()
        if cpu_probe.status == "passed" and cpu_value:
            cpu.update({"model": cpu_value, "model_source": "sysctl machdep.cpu.brand_string"})
        power_probe = run.run_command(
            CommandSpec("power-mode", ("pmset", "-g", "custom"), run.repo_root),
            required=False,
        )
        if power_probe.status == "passed":
            entries = [
                line.strip()
                for line in power_probe.stdout.decode("utf-8", errors="replace").splitlines()
                if "lowpowermode" in line
            ]
            if entries:
                power = {
                    "availability": "available",
                    "value": entries,
                    "source": "pmset -g custom",
                }
    run.environment = {
        "schema_version": SCHEMA_VERSION,
        "collection_status": "complete",
        "runner": {
            "label": run.runner_label,
            "github": allowlisted_runner_environment(),
        },
        "platform": {
            "os": platform.system(),
            "release": platform.release(),
            "kernel": platform.version(),
            "architecture": platform.machine(),
        },
        "cpu": cpu,
        "power": power,
        "tools": {
            "rustc": parse_rustc_verbose(rustc.stdout.decode("utf-8", errors="replace")),
            "cargo": cargo.stdout.decode("utf-8", errors="replace").strip(),
            "python": {
                "version": platform.python_version(),
                "implementation": platform.python_implementation(),
                "executable": sys.executable,
            },
        },
        "cargo_metadata": metadata_summary,
    }
    write_json(run.run_dir / "environment.json", run.environment)
    return bench_targets, target_directory


def correctness_plan(repo_root: Path) -> list[CommandSpec]:
    python_crate = repo_root / "crates/rusty-modbus-python"
    return [
        CommandSpec("cargo-fmt", ("cargo", "fmt", "--all", "--check"), repo_root),
        CommandSpec(
            "conformance-ledger-tests",
            ("python3", "scripts/test-conformance-ledger.py"),
            repo_root,
        ),
        CommandSpec(
            "conformance-ledger-check",
            ("python3", "scripts/check-conformance-ledger.py", "--check"),
            repo_root,
        ),
        CommandSpec("baseline-tests", ("python3", "scripts/test-baseline.py"), repo_root),
        CommandSpec(
            "workspace-clippy",
            (
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ),
            repo_root,
        ),
        CommandSpec(
            "workspace-nextest",
            ("cargo", "nextest", "run", "--workspace", "--locked", "--profile", "ci"),
            repo_root,
        ),
        CommandSpec(
            "workspace-doctests",
            ("cargo", "test", "--workspace", "--locked", "--doc"),
            repo_root,
        ),
        CommandSpec(
            "conformance-nextest",
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
            repo_root,
        ),
        CommandSpec(
            "facade-no-default",
            (
                "cargo",
                "check",
                "-p",
                "rusty-modbus",
                "--no-default-features",
                "--locked",
            ),
            repo_root,
        ),
        CommandSpec(
            "facade-full",
            ("cargo", "check", "-p", "rusty-modbus", "--features", "full", "--locked"),
            repo_root,
        ),
        CommandSpec(
            "facade-all-features",
            ("cargo", "check", "-p", "rusty-modbus", "--all-features", "--locked"),
            repo_root,
        ),
        CommandSpec(
            "types-no-std",
            (
                "cargo",
                "check",
                "-p",
                "rusty-modbus-types",
                "--no-default-features",
                "--locked",
            ),
            repo_root,
        ),
        CommandSpec(
            "codec-no-std",
            (
                "cargo",
                "check",
                "-p",
                "rusty-modbus-codec",
                "--no-default-features",
                "--locked",
            ),
            repo_root,
        ),
        CommandSpec(
            "publish-version-check",
            ("python3", "scripts/check-publish-versions.py"),
            repo_root,
        ),
        CommandSpec(
            "full-feature-examples",
            (
                "cargo",
                "check",
                "-p",
                "rusty-modbus",
                "--examples",
                "--features",
                "full",
                "--locked",
            ),
            repo_root,
        ),
        CommandSpec(
            "python-binding-clippy",
            ("cargo", "clippy", "--all-targets", "--locked", "--", "-D", "warnings"),
            python_crate,
        ),
        CommandSpec("python-binding-full", ("scripts/ci-python.sh",), repo_root),
        CommandSpec(
            "cargo-deny",
            ("cargo", "deny", "check", "bans", "licenses", "sources"),
            repo_root,
        ),
        CommandSpec(
            "cargo-audit",
            ("cargo", "audit", "--ignore", "RUSTSEC-2025-0134"),
            repo_root,
        ),
    ]


def stress_scenarios(mode: str, repetitions: int) -> list[dict[str, Any]]:
    depths = (1, 8, 16) if mode == "bench-smoke" else (1, 2, 4, 8, 16)
    return [
        {
            "transport": "tcp",
            "operation": operation,
            "in_flight": depth,
            "clients": 1,
            "registers": 10,
            "repetition": repetition,
        }
        for operation in ("read", "mixed")
        for depth in depths
        for repetition in range(1, repetitions + 1)
    ]


def benchmark_criterion_specs(
    mode: str, repo_root: Path, bench_targets: Sequence[str], run_dir: Path
) -> list[CommandSpec]:
    if mode == "bench-smoke":
        definitions = (("tcp_throughput", "tcp_pipelined"),)
    else:
        definitions = tuple(
            (target, None)
            for target in sorted(bench_targets)
            if target.startswith("tcp_")
        )
    specs = []
    for index, (target, bench_filter) in enumerate(definitions, 1):
        argv = [
            "cargo",
            "bench",
            "-p",
            "rusty-modbus-benchmarks",
            "--bench",
            target,
            "--locked",
        ]
        if bench_filter:
            argv.append(bench_filter)
        argv.extend(("--", "--quick", "--noplot"))
        criterion_home = run_dir / "criterion" / "raw" / f"{index:02d}-{target}"
        specs.append(
            CommandSpec(
                f"criterion-{target}",
                tuple(argv),
                repo_root,
                (("CRITERION_HOME", str(criterion_home)),),
            )
        )
    return specs


def _strict_int(value: Any, label: str, *, minimum: int = 0) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        raise BaselineError(f"{label} must be an integer >= {minimum}")
    return value


def _strict_number(value: Any, label: str, *, minimum: float = 0.0) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise BaselineError(f"{label} must be numeric")
    try:
        result = float(value)
    except (OverflowError, ValueError) as error:
        raise BaselineError(f"{label} must be finite and >= {minimum}") from error
    if not math.isfinite(result) or result < minimum:
        raise BaselineError(f"{label} must be finite and >= {minimum}")
    return result


def parse_stress_json(payload: bytes | str, expected: dict[str, Any]) -> dict[str, Any]:
    try:
        value = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise BaselineError(f"stress JSON is malformed: {error}") from error
    if not isinstance(value, dict):
        raise BaselineError("stress JSON root must be an object")
    for field in STRESS_INTEGER_FIELDS:
        minimum = (
            1
            if field in {"clients", "in_flight", "duration_secs", "registers", "total_ops"}
            else 0
        )
        _strict_int(value.get(field), f"stress.{field}", minimum=minimum)
    if value["schema_version"] != SCHEMA_VERSION:
        raise BaselineError(f"stress.schema_version must be {SCHEMA_VERSION}")
    for field in STRESS_NUMERIC_FIELDS:
        _strict_number(value.get(field), f"stress.{field}")
    if not isinstance(value.get("transport"), str) or not isinstance(
        value.get("operation"), str
    ):
        raise BaselineError("stress transport and operation must be strings")
    latency = value.get("latency_ms")
    if not isinstance(latency, dict):
        raise BaselineError("stress.latency_ms must be an object")
    for field in LATENCY_FIELDS:
        _strict_number(latency.get(field), f"stress.latency_ms.{field}")
    memory = value.get("memory")
    if not isinstance(memory, dict):
        raise BaselineError("stress.memory must be an object")
    for field in MEMORY_FIELDS:
        minimum = -float("inf") if field == "delta_mb" else 0
        number = memory.get(field)
        if isinstance(number, bool) or not isinstance(number, int):
            raise BaselineError(f"stress.memory.{field} must be an integer")
        if number < minimum:
            raise BaselineError(f"stress.memory.{field} is out of range")
    for field in (
        "transport",
        "operation",
        "in_flight",
        "clients",
        "registers",
        "duration_secs",
        "warmup_secs",
    ):
        if value.get(field) != expected[field]:
            raise BaselineError(
                f"stress scenario mismatch for {field}: expected {expected[field]!r}, "
                f"got {value.get(field)!r}"
            )
    if value["errors"] != 0:
        raise BaselineError("stress errors must be zero")
    if value["error_rate"] != 0:
        raise BaselineError("stress error_rate must be zero")
    if value["retry_attempts"] != 0:
        raise BaselineError("stress retry_attempts must be zero")
    return value


def _stress_selector_key(
    value: dict[str, Any], label: str, *, include_repetition: bool
) -> tuple[Any, ...]:
    transport = _require_nonempty_string(value.get("transport"), f"{label}.transport")
    operation = _require_nonempty_string(value.get("operation"), f"{label}.operation")
    if transport != "tcp" or operation not in {"read", "mixed"}:
        raise BaselineError(f"{label} must use a supported TCP scenario")
    key = (
        transport,
        operation,
        _strict_int(value.get("in_flight"), f"{label}.in_flight", minimum=1),
        _strict_int(value.get("clients"), f"{label}.clients", minimum=1),
        _strict_int(value.get("registers"), f"{label}.registers", minimum=1),
    )
    if include_repetition:
        return key + (
            _strict_int(value.get("repetition"), f"{label}.repetition", minimum=1),
        )
    return key


def sample_statistics(values: Sequence[float]) -> dict[str, float | int | None]:
    if not values:
        raise BaselineError("cannot aggregate an empty sample")
    converted = [float(value) for value in values]
    mean = statistics.fmean(converted)
    standard_deviation = statistics.stdev(converted) if len(converted) > 1 else 0.0
    return {
        "count": len(converted),
        "min": min(converted),
        "median": statistics.median(converted),
        "mean": mean,
        "max": max(converted),
        "sample_stddev": standard_deviation,
        "coefficient_of_variation": None if mean == 0 else standard_deviation / mean,
    }


def aggregate_stress_samples(
    samples: Sequence[dict[str, Any]], expected_scenarios: Sequence[dict[str, Any]]
) -> list[dict[str, Any]]:
    expected_keys = {
        _stress_selector_key(
            item, "expected stress scenario", include_repetition=True
        )
        for item in expected_scenarios
    }
    sample_keys = [
        _stress_selector_key(item, "stress sample", include_repetition=True)
        for item in samples
    ]
    duplicates = sorted({item for item in sample_keys if sample_keys.count(item) > 1})
    if duplicates:
        raise BaselineError(f"duplicate stress scenarios: {duplicates}")
    missing = sorted(expected_keys - set(sample_keys))
    extra = sorted(set(sample_keys) - expected_keys)
    if missing or extra:
        raise BaselineError(f"stress scenario completeness failure; missing={missing}, extra={extra}")
    groups: dict[tuple[Any, ...], list[dict[str, Any]]] = {}
    for sample in samples:
        key = _stress_selector_key(
            sample, "stress sample", include_repetition=False
        )
        groups.setdefault(key, []).append(sample)
    aggregates = []
    for key in sorted(groups):
        group = groups[key]
        aggregates.append(
            {
                "transport": key[0],
                "operation": key[1],
                "in_flight": key[2],
                "clients": key[3],
                "registers": key[4],
                "repetitions": len(group),
                "throughput_ops_sec": sample_statistics(
                    [item["throughput_ops_sec"] for item in group]
                ),
                "p99_ms": sample_statistics([item["latency_ms"]["p99"] for item in group]),
                "total_errors": sum(item["errors"] for item in group),
                "retry_attempts": sum(item["retry_attempts"] for item in group),
            }
        )
    return aggregates


def parse_criterion_estimates(criterion_home: Path, repo_root: Path) -> list[dict[str, Any]]:
    estimate_files = sorted(
        criterion_home.rglob("new/estimates.json"),
        key=lambda path: path.relative_to(criterion_home).as_posix().encode("utf-8"),
    )
    if not estimate_files:
        raise BaselineError(f"no Criterion new/estimates.json files found under {criterion_home}")
    results = []
    for path in estimate_files:
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise BaselineError(f"malformed Criterion estimates file {path}: {error}") from error
        mean = value.get("mean") if isinstance(value, dict) else None
        if not isinstance(mean, dict):
            raise BaselineError(f"Criterion mean estimate missing in {path}")
        interval = mean.get("confidence_interval")
        if not isinstance(interval, dict):
            raise BaselineError(f"Criterion mean confidence interval missing in {path}")
        point = _strict_number(mean.get("point_estimate"), f"{path}.mean.point_estimate")
        lower = _strict_number(interval.get("lower_bound"), f"{path}.mean.lower_bound")
        upper = _strict_number(interval.get("upper_bound"), f"{path}.mean.upper_bound")
        benchmark_path = path.relative_to(criterion_home).parents[1].as_posix()
        results.append(
            {
                "benchmark_id": benchmark_path,
                "source": path.resolve().relative_to(repo_root.resolve()).as_posix(),
                "mean_ns": {"lower": lower, "point": point, "upper": upper},
                "estimates": value,
            }
        )
    return results


def write_summary_csv(
    path: Path,
    stress_aggregates: Sequence[dict[str, Any]],
    criterion_results: Sequence[dict[str, Any]],
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as handle:
        columns: list[str] = list(CSV_COLUMNS)
        writer = csv.DictWriter(handle, fieldnames=columns, lineterminator="\n")
        writer.writeheader()
        for item in stress_aggregates:
            throughput = item["throughput_ops_sec"]
            p99 = item["p99_ms"]
            row = {column: "" for column in CSV_COLUMNS}
            row.update(
                {
                    "record_type": "stress",
                    "transport": item["transport"],
                    "operation": item["operation"],
                    "in_flight": item["in_flight"],
                    "clients": item["clients"],
                    "registers": item["registers"],
                    "repetitions": item["repetitions"],
                    "throughput_count": throughput["count"],
                    "throughput_min": throughput["min"],
                    "throughput_median": throughput["median"],
                    "throughput_mean": throughput["mean"],
                    "throughput_max": throughput["max"],
                    "throughput_sample_stddev": throughput["sample_stddev"],
                    "throughput_cv": throughput["coefficient_of_variation"],
                    "p99_count": p99["count"],
                    "p99_min_ms": p99["min"],
                    "p99_median_ms": p99["median"],
                    "p99_mean_ms": p99["mean"],
                    "p99_max_ms": p99["max"],
                    "p99_sample_stddev_ms": p99["sample_stddev"],
                    "p99_cv": p99["coefficient_of_variation"],
                    "total_errors": item["total_errors"],
                    "retry_attempts": item["retry_attempts"],
                }
            )
            writer.writerow(row)
        for item in criterion_results:
            row = {column: "" for column in CSV_COLUMNS}
            row.update(
                {
                    "record_type": "criterion",
                    "criterion_id": item["benchmark_id"],
                    "criterion_mean_lower_ns": item["mean_ns"]["lower"],
                    "criterion_mean_point_ns": item["mean_ns"]["point"],
                    "criterion_mean_upper_ns": item["mean_ns"]["upper"],
                }
            )
            writer.writerow(row)


def _artifact_files(run_dir: Path) -> list[Path]:
    return sorted(
        (
            path
            for path in run_dir.rglob("*")
            if path.is_file() and path.name != "checksums.sha256"
        ),
        key=lambda path: path.as_posix().encode("utf-8"),
    )


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def write_checksums(repo_root: Path, run_dir: Path) -> None:
    lines = []
    for path in _artifact_files(run_dir):
        relative = path.resolve().relative_to(repo_root.resolve()).as_posix()
        lines.append(f"{_sha256(path)}  {relative}")
    lines.sort(key=lambda line: line.split("  ", 1)[1].encode("utf-8"))
    (run_dir / "checksums.sha256").write_text(
        "\n".join(lines) + "\n", encoding="utf-8", newline="\n"
    )


def validate_artifact(repo_root: Path, run_dir: Path) -> list[str]:
    errors: list[str] = []
    repo_root = repo_root.resolve()
    run_dir = run_dir.resolve()
    try:
        run_dir.relative_to(repo_root)
    except ValueError:
        return ["artifact directory must be inside the repository"]
    if not run_dir.is_dir():
        return [f"artifact directory does not exist: {run_dir}"]
    if run_dir.parents[1].name != f"baseline-v{SCHEMA_VERSION}":
        errors.append(f"artifact path must be under baseline-v{SCHEMA_VERSION}/<SHA>/<run-id>")
    try:
        path_sha = validate_full_sha(run_dir.parent.name)
    except BaselineError as error:
        errors.append(str(error))
        path_sha = None
    try:
        validate_run_id(run_dir.name)
    except BaselineError as error:
        errors.append(str(error))
    required = {"environment.json", "provenance.json", "summary.json", "summary.csv"}
    present = {path.name for path in run_dir.iterdir() if path.is_file()}
    missing_required = sorted(required - present)
    if missing_required:
        errors.append(f"missing required artifact files: {', '.join(missing_required)}")
    checksum_path = run_dir / "checksums.sha256"
    if not checksum_path.is_file():
        errors.append("missing checksums.sha256")
        return errors
    try:
        checksum_lines = checksum_path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as error:
        errors.append(f"cannot read checksums.sha256: {error}")
        return errors
    listed: dict[str, str] = {}
    listed_order: list[str] = []
    for line_number, line in enumerate(checksum_lines, 1):
        parts = line.split("  ", 1)
        if len(parts) != 2 or not re.fullmatch(r"[0-9a-f]{64}", parts[0]):
            errors.append(f"invalid checksum line {line_number}")
            continue
        digest, relative = parts
        if relative in listed:
            errors.append(f"duplicate checksum path: {relative}")
            continue
        path = (repo_root / relative).resolve()
        try:
            path.relative_to(run_dir)
        except ValueError:
            errors.append(f"checksum path escapes artifact directory: {relative}")
            continue
        listed[relative] = digest
        listed_order.append(relative)
        if not path.is_file():
            errors.append(f"checksummed file is missing: {relative}")
        elif _sha256(path) != digest:
            errors.append(f"checksum mismatch: {relative}")
    actual = {
        path.resolve().relative_to(repo_root).as_posix() for path in _artifact_files(run_dir)
    }
    listed_paths = set(listed)
    if listed_order != sorted(listed_order, key=lambda item: item.encode("utf-8")):
        errors.append("checksum paths are not in bytewise order")
    if listed_paths != actual:
        errors.append(
            f"checksum inventory mismatch; missing={sorted(actual - listed_paths)}, "
            f"extra={sorted(listed_paths - actual)}"
        )
    documents: dict[str, dict[str, Any]] = {}
    for name in ("environment.json", "provenance.json", "summary.json"):
        try:
            value = json.loads((run_dir / name).read_text(encoding="utf-8"))
            if not isinstance(value, dict):
                raise BaselineError("root must be an object")
            documents[name] = value
            if value.get("schema_version") != SCHEMA_VERSION:
                errors.append(f"{name} schema_version must be {SCHEMA_VERSION}")
        except (OSError, UnicodeDecodeError, json.JSONDecodeError, BaselineError) as error:
            errors.append(f"cannot validate {name}: {error}")

    provenance = documents.get("provenance.json")
    summary = documents.get("summary.json")
    if provenance is None or summary is None:
        return errors

    for name, value in (("provenance.json", provenance), ("summary.json", summary)):
        target_sha = value.get("target_sha")
        if not isinstance(target_sha, str) or not FULL_SHA.fullmatch(target_sha):
            errors.append(f"{name} target_sha must be a full lowercase SHA")
        elif path_sha is not None and target_sha != path_sha:
            errors.append(f"{name} target_sha does not match artifact path")

        run_id = value.get("run_id")
        if run_id != run_dir.name:
            errors.append(f"{name} run_id does not match artifact path")

        mode = value.get("mode")
        if not isinstance(mode, str) or not mode:
            errors.append(f"{name} mode must be a non-empty string")
        elif mode not in MODES:
            errors.append(f"{name} mode must be one of {', '.join(MODES)}")

    if provenance.get("target_sha") != summary.get("target_sha"):
        errors.append("provenance.json and summary.json target_sha do not match")
    if provenance.get("run_id") != summary.get("run_id"):
        errors.append("provenance.json and summary.json run_id do not match")
    if provenance.get("mode") != summary.get("mode"):
        errors.append("provenance.json and summary.json mode do not match")

    dirty = provenance.get("dirty")
    dirty_override = provenance.get("dirty_override")
    baseline_eligible = provenance.get("baseline_eligible")
    baseline_valid = summary.get("baseline_valid")
    status = summary.get("status")
    invalid_reasons = summary.get("invalid_reasons")

    if not isinstance(dirty, bool):
        errors.append("provenance.json dirty must be a boolean")
    if not isinstance(dirty_override, bool):
        errors.append("provenance.json dirty_override must be a boolean")
    if not isinstance(baseline_eligible, bool):
        errors.append("provenance.json baseline_eligible must be a boolean")
    if not isinstance(baseline_valid, bool):
        errors.append("summary.json baseline_valid must be a boolean")
    status_is_string = isinstance(status, str) and bool(status)
    if not status_is_string:
        errors.append("summary.json status must be a non-empty string")
    elif status not in {"passed", "failed", "invalid"}:
        errors.append("summary.json status must be passed, failed, or invalid")
    if not isinstance(invalid_reasons, list) or any(
        not isinstance(reason, str) for reason in invalid_reasons
    ):
        errors.append("summary.json invalid_reasons must be a list of strings")

    if dirty is True and dirty_override is not True:
        errors.append("provenance.json dirty worktree requires dirty_override")
    if isinstance(baseline_eligible, bool) and isinstance(baseline_valid, bool):
        if baseline_eligible != baseline_valid:
            errors.append("provenance eligibility and summary validity do not match")
    if (
        isinstance(baseline_valid, bool)
        and status_is_string
        and status in {"passed", "failed", "invalid"}
    ):
        if baseline_valid != (status == "passed"):
            errors.append("summary.json status does not agree with baseline_valid")
    if baseline_valid is True and invalid_reasons != []:
        errors.append("valid summary.json must not contain invalid_reasons")
    if baseline_valid is False and invalid_reasons == []:
        errors.append("invalid summary.json must contain invalid_reasons")
    if status == "invalid" and dirty is not True:
        errors.append("summary.json invalid status requires a dirty worktree")

    if dirty is True:
        errors.append("provenance.json records a dirty worktree")
    if baseline_eligible is False:
        errors.append("provenance.json baseline_eligible must be true")
    if baseline_valid is False:
        errors.append("summary.json baseline_valid must be true")
    return errors


def _read_json_object(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise BaselineError(f"cannot read {label}: {error}") from error
    if not isinstance(value, dict):
        raise BaselineError(f"{label} root must be an object")
    return value


def _relative_parts(value: str, label: str) -> tuple[str, ...]:
    if not isinstance(value, str) or not value:
        raise BaselineError(f"{label} must be a non-empty relative path")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        raise BaselineError(f"{label} must be a traversal-free relative path")
    return path.parts


def _reject_symlink_components(path: Path, root: Path, label: str) -> None:
    root = root.resolve()
    try:
        relative = path.relative_to(root)
    except ValueError as error:
        raise BaselineError(f"{label} must be inside the repository") from error
    current = root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise BaselineError(f"{label} must not use symlinks: {current}")


def _artifact_file(run_dir: Path, relative: str, label: str) -> tuple[Path, str]:
    parts = _relative_parts(relative, label)
    path = run_dir.joinpath(*parts)
    _reject_symlink_components(path, run_dir, label)
    if not path.is_file():
        raise BaselineError(f"{label} is missing: {relative}")
    return path, PurePosixPath(*parts).as_posix()


def _stored_artifact_file(
    repo_root: Path, run_dir: Path, value: Any, label: str
) -> tuple[Path, str]:
    parts = _relative_parts(value, label)
    path = repo_root.joinpath(*parts)
    try:
        relative = path.relative_to(run_dir)
    except ValueError as error:
        raise BaselineError(f"{label} must stay inside the source artifact") from error
    return _artifact_file(run_dir, relative.as_posix(), label)


def _require_nonempty_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise BaselineError(f"{label} must be a non-empty string")
    return value


def criterion_version_from_target_lock(repo_root: Path, target_sha: str) -> str:
    target_sha = validate_full_sha(target_sha)
    try:
        completed = subprocess.run(
            ("git", "cat-file", "blob", f"{target_sha}:Cargo.lock"),
            cwd=repo_root,
            capture_output=True,
            check=False,
            shell=False,
        )
    except OSError as error:
        raise BaselineError(f"cannot read target-SHA Cargo.lock evidence: {error}") from error
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", errors="replace").strip()
        raise BaselineError(
            f"target-SHA Cargo.lock evidence is unavailable for {target_sha}: {detail}"
        )
    try:
        lock = tomllib.loads(completed.stdout.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise BaselineError(f"target-SHA Cargo.lock evidence is malformed: {error}") from error
    packages = lock.get("package")
    if not isinstance(packages, list):
        raise BaselineError("target-SHA Cargo.lock package inventory is unavailable")
    matches = [
        package
        for package in packages
        if isinstance(package, dict) and package.get("name") == "criterion"
    ]
    if not matches:
        raise BaselineError("target-SHA Cargo.lock does not identify Criterion")
    if len(matches) != 1:
        raise BaselineError("target-SHA Cargo.lock Criterion evidence is ambiguous")
    version = matches[0].get("version")
    if not isinstance(version, str) or not version:
        raise BaselineError("target-SHA Cargo.lock Criterion version is unlabelled")
    return version


def verified_criterion_version(
    repo_root: Path, target_sha: str, *, declared_version: str | None = None
) -> str:
    locked_version = criterion_version_from_target_lock(repo_root, target_sha)
    if declared_version is not None and declared_version != locked_version:
        raise BaselineError(
            "report Criterion version does not match target-SHA Cargo.lock: "
            f"report={declared_version!r}, lock={locked_version!r}"
        )
    if locked_version != SUPPORTED_CRITERION_VERSION:
        raise BaselineError(
            "target-SHA Cargo.lock Criterion version is unsupported: "
            f"{locked_version!r}"
        )
    return locked_version


def _report_producers(
    *, stress: bool, criterion: bool, criterion_version: str | None = None
) -> list[dict[str, str]]:
    producers = [
        {
            "adapter": "baseline artifact schema v1",
            "id": "rusty-modbus-baseline-harness-v1",
            "producer": HARNESS_PATH,
            "version": HARNESS_VERSION,
        }
    ]
    if stress:
        producers.append(
            {
                "adapter": "custom stress JSON schema v1",
                "id": STRESS_PRODUCER_ID,
                "producer": "rusty-modbus stress-test",
                "version": "1",
            }
        )
    if criterion:
        if criterion_version is None:
            raise BaselineError("verified Criterion producer version is required")
        producers.append(
            {
                "adapter": CRITERION_ADAPTER,
                "id": CRITERION_PRODUCER_ID,
                "producer": "Criterion",
                "version": criterion_version,
            }
        )
    return producers


def _validate_report_environment(environment: dict[str, Any]) -> dict[str, Any]:
    if environment.get("schema_version") != SCHEMA_VERSION:
        raise BaselineError(f"environment.json schema_version must be {SCHEMA_VERSION}")
    if environment.get("collection_status") != "complete":
        raise BaselineError("environment.json collection_status must be complete")
    runner = environment.get("runner")
    if not isinstance(runner, dict):
        raise BaselineError("environment.json runner must be an object")
    label = validate_runner_label(
        _require_nonempty_string(runner.get("label"), "environment.json runner.label")
    )
    github = runner.get("github")
    if not isinstance(github, dict) or any(
        not isinstance(key, str) or not isinstance(value, str)
        for key, value in github.items()
    ):
        raise BaselineError("environment.json runner.github must contain string values")
    for field in ("platform", "cpu", "power", "tools", "cargo_metadata"):
        if not isinstance(environment.get(field), dict):
            raise BaselineError(f"environment.json {field} must be an object")
    tools = environment["tools"]
    rustc = tools.get("rustc")
    python = tools.get("python")
    if not isinstance(rustc, dict) or not isinstance(python, dict):
        raise BaselineError("environment.json rustc and python tools must be objects")
    _require_nonempty_string(rustc.get("version"), "environment.json rustc.version")
    _require_nonempty_string(rustc.get("host"), "environment.json rustc.host")
    _require_nonempty_string(tools.get("cargo"), "environment.json tools.cargo")
    for field in ("version", "implementation", "executable"):
        _require_nonempty_string(
            python.get(field), f"environment.json tools.python.{field}"
        )
    return {
        "cargo_metadata": environment["cargo_metadata"],
        "collection_status": "complete",
        "cpu": environment["cpu"],
        "github": dict(sorted(github.items())),
        "platform": environment["platform"],
        "power": environment["power"],
        "tools": tools,
        "runner_label": label,
    }


def _report_stress_scenarios(
    repo_root: Path,
    run_dir: Path,
    mode: str,
    summary: dict[str, Any],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    samples = summary.get("stress_samples")
    aggregates = summary.get("stress_aggregates")
    if not isinstance(samples, list) or not samples:
        raise BaselineError("summary.json stress_samples must be a non-empty list")
    if not isinstance(aggregates, list) or not aggregates:
        raise BaselineError("summary.json stress_aggregates must be a non-empty list")
    if any(not isinstance(item, dict) for item in samples + aggregates):
        raise BaselineError("summary.json stress entries must be objects")

    durations: set[int] = set()
    warmups: set[int] = set()
    repetitions: list[int] = []
    sample_sources: dict[tuple[Any, ...], list[dict[str, Any]]] = {}
    summary_commands = summary.get("commands")
    if not isinstance(summary_commands, list) or any(
        not isinstance(item, str) for item in summary_commands
    ):
        raise BaselineError("summary.json commands must be a list of strings")

    for sample in samples:
        selector_key = _stress_selector_key(sample, "stress", include_repetition=True)
        expected = {
            field: sample.get(field)
            for field in (
                "transport",
                "operation",
                "in_flight",
                "clients",
                "registers",
                "duration_secs",
                "warmup_secs",
            )
        }
        parse_stress_json(json.dumps(sample, allow_nan=False), expected)
        repetition = selector_key[-1]
        command_id = sample.get("command_id")
        if not isinstance(command_id, str) or not COMMAND_ID.fullmatch(command_id):
            raise BaselineError("stress.command_id is missing or malformed")
        if command_id not in summary_commands:
            raise BaselineError(f"stress command is absent from summary.json: {command_id}")

        durations.add(sample["duration_secs"])
        warmups.add(sample["warmup_secs"])
        repetitions.append(repetition)
        parsed_relative = (
            f"stress/parsed/stress-{sample['operation']}-d{sample['in_flight']}-"
            f"r{repetition}.json"
        )
        parsed_path, parsed_source = _artifact_file(
            run_dir, parsed_relative, "stress parsed source"
        )
        if _read_json_object(parsed_path, parsed_relative) != sample:
            raise BaselineError(f"stress parsed source does not match summary: {parsed_relative}")

        command_relative = f"commands/{command_id}/command.json"
        command_path, command_source = _artifact_file(
            run_dir, command_relative, "stress command record"
        )
        command = _read_json_object(command_path, command_relative)
        if (
            command.get("schema_version") != SCHEMA_VERSION
            or command.get("command_id") != command_id
            or command.get("status") != "passed"
        ):
            raise BaselineError(f"stress command record is not a passed v1 record: {command_id}")
        stdout_path, stdout_source = _stored_artifact_file(
            repo_root,
            run_dir,
            command.get("stdout_path"),
            f"{command_id} stdout_path",
        )
        expected_stdout = run_dir / "commands" / command_id / "command.stdout"
        if stdout_path != expected_stdout:
            raise BaselineError(f"stress command stdout path mismatch: {command_id}")
        raw_sample = parse_stress_json(stdout_path.read_bytes(), expected)
        summary_raw_sample = dict(sample)
        summary_raw_sample.pop("command_id")
        summary_raw_sample.pop("repetition")
        if raw_sample != summary_raw_sample:
            raise BaselineError(f"stress raw stdout does not match parsed sample: {command_id}")

        key = selector_key[:-1]
        sample_sources.setdefault(key, []).append(
            {
                "command_record": command_source,
                "parsed_sample": parsed_source,
                "raw_stdout": stdout_source,
                "repetition": repetition,
            }
        )

    if len(durations) != 1 or len(warmups) != 1:
        raise BaselineError("stress samples must use one duration and warmup")
    repetition_count = max(repetitions)
    expected_scenarios = stress_scenarios(mode, repetition_count)
    canonical_aggregates = aggregate_stress_samples(samples, expected_scenarios)
    if aggregates != canonical_aggregates:
        raise BaselineError("summary.json stress_aggregates do not match strict sample aggregation")

    scenarios = []
    for aggregate in canonical_aggregates:
        key = (
            aggregate["transport"],
            aggregate["operation"],
            aggregate["in_flight"],
            aggregate["clients"],
            aggregate["registers"],
        )
        sources = sorted(sample_sources[key], key=lambda item: item["repetition"])
        scenarios.append(
            {
                "correctness": {
                    "error_rate": 0.0,
                    "retry_attempts": aggregate["retry_attempts"],
                    "total_errors": aggregate["total_errors"],
                    "zero_error_rate": True,
                    "zero_errors": aggregate["total_errors"] == 0,
                    "zero_retries": aggregate["retry_attempts"] == 0,
                },
                "identity": {
                    "clients": aggregate["clients"],
                    "duration_seconds": next(iter(durations)),
                    "in_flight": aggregate["in_flight"],
                    "operation": aggregate["operation"],
                    "registers": aggregate["registers"],
                    "repetitions": aggregate["repetitions"],
                    "transport": aggregate["transport"],
                    "warmup_seconds": next(iter(warmups)),
                },
                "kind": "tcp_stress",
                "metrics": {
                    "p99_latency": {
                        "recorded_statistics": aggregate["p99_ms"],
                        "unit": "milliseconds",
                    },
                    "throughput": {
                        "recorded_statistics": aggregate["throughput_ops_sec"],
                        "unit": "operations_per_second",
                    },
                },
                "producer_id": STRESS_PRODUCER_ID,
                "sources": sources,
            }
        )
    correctness = {
        "stress_sample_count": len(samples),
        "total_errors": sum(item["errors"] for item in samples),
        "total_retry_attempts": sum(item["retry_attempts"] for item in samples),
        "zero_errors": all(item["errors"] == 0 and item["error_rate"] == 0 for item in samples),
        "zero_retries": all(item["retry_attempts"] == 0 for item in samples),
    }
    return scenarios, correctness


def _report_criterion_scenarios(
    repo_root: Path, run_dir: Path, summary: dict[str, Any]
) -> list[dict[str, Any]]:
    results = summary.get("criterion_results")
    if not isinstance(results, list) or not results:
        raise BaselineError("summary.json criterion_results must be a non-empty list")
    if any(not isinstance(item, dict) for item in results):
        raise BaselineError("summary.json Criterion entries must be objects")
    parsed_path, _ = _artifact_file(
        run_dir, "criterion/parsed-estimates.json", "Criterion parsed estimates"
    )
    try:
        parsed_results = json.loads(parsed_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise BaselineError(f"cannot read Criterion parsed estimates: {error}") from error
    if parsed_results != results:
        raise BaselineError("Criterion parsed estimates do not match summary.json")

    seen_sources: set[str] = set()
    scenarios = []
    for result in sorted(
        results, key=lambda item: (str(item.get("benchmark_id")), str(item.get("source")))
    ):
        benchmark_id = _require_nonempty_string(
            result.get("benchmark_id"), "Criterion benchmark_id"
        )
        _relative_parts(benchmark_id, "Criterion benchmark_id")
        source_path, source = _stored_artifact_file(
            repo_root, run_dir, result.get("source"), "Criterion estimates source"
        )
        if source in seen_sources:
            raise BaselineError(f"duplicate Criterion source: {source}")
        seen_sources.add(source)
        source_parts = PurePosixPath(source).parts
        if (
            len(source_parts) < 6
            or source_parts[:2] != ("criterion", "raw")
            or source_parts[-2:] != ("new", "estimates.json")
            or "/".join(source_parts[3:-2]) != benchmark_id
        ):
            raise BaselineError(
                "Criterion source must match the exact new/estimates.json private layout"
            )
        estimates = result.get("estimates")
        if not isinstance(estimates, dict) or _read_json_object(
            source_path, f"Criterion source {source}"
        ) != estimates:
            raise BaselineError(f"Criterion source does not match parsed estimates: {source}")
        mean = estimates.get("mean")
        interval = mean.get("confidence_interval") if isinstance(mean, dict) else None
        if not isinstance(mean, dict) or not isinstance(interval, dict):
            raise BaselineError(f"Criterion mean estimate is missing: {source}")
        point = _strict_number(mean.get("point_estimate"), f"{source}.mean.point_estimate")
        lower = _strict_number(interval.get("lower_bound"), f"{source}.mean.lower_bound")
        upper = _strict_number(interval.get("upper_bound"), f"{source}.mean.upper_bound")
        confidence_level = _strict_number(
            interval.get("confidence_level"), f"{source}.mean.confidence_level"
        )
        standard_error = _strict_number(
            mean.get("standard_error"), f"{source}.mean.standard_error"
        )
        if not 0 < confidence_level <= 1:
            raise BaselineError(f"Criterion confidence level is out of range: {source}")
        if not lower <= point <= upper:
            raise BaselineError(f"Criterion mean estimate bounds are out of order: {source}")
        normalized = result.get("mean_ns")
        if not isinstance(normalized, dict) or normalized != {
            "lower": lower,
            "point": point,
            "upper": upper,
        }:
            raise BaselineError(f"Criterion normalized mean does not match source: {source}")
        scenarios.append(
            {
                "identity": {"benchmark_id": benchmark_id},
                "kind": "criterion_estimate",
                "metrics": {
                    "mean_estimate": {
                        "confidence_level": confidence_level,
                        "lower": lower,
                        "point": point,
                        "standard_error": standard_error,
                        "unit": "nanoseconds",
                        "upper": upper,
                    }
                },
                "producer_id": CRITERION_PRODUCER_ID,
                "sources": [{"private_estimates_json": source}],
            }
        )
    return scenarios


def build_benchmark_report(
    repo_root: Path,
    run_dir: Path,
    *,
    require_artifact_checksums: bool = True,
) -> dict[str, Any]:
    repo_root = repo_root.resolve()
    run_dir = run_dir.resolve()
    try:
        artifact_path = run_dir.relative_to(repo_root).as_posix()
    except ValueError as error:
        raise BaselineError("artifact directory must be inside the repository") from error
    if require_artifact_checksums:
        errors = validate_artifact(repo_root, run_dir)
        if errors:
            raise BaselineError("artifact validation failed: " + "; ".join(errors))

    environment = _read_json_object(run_dir / "environment.json", "environment.json")
    provenance = _read_json_object(run_dir / "provenance.json", "provenance.json")
    summary = _read_json_object(run_dir / "summary.json", "summary.json")
    for name, document in (
        ("environment.json", environment),
        ("provenance.json", provenance),
        ("summary.json", summary),
    ):
        if document.get("schema_version") != SCHEMA_VERSION:
            raise BaselineError(f"{name} schema_version must be {SCHEMA_VERSION}")
    if (
        provenance.get("harness_version") != HARNESS_VERSION
        or provenance.get("harness_path") != HARNESS_PATH
    ):
        raise BaselineError("unsupported or unlabelled artifact harness producer version")
    if provenance.get("dirty") is not False or provenance.get("baseline_eligible") is not True:
        raise BaselineError("report source must record a clean, eligible worktree")
    if (
        summary.get("status") != "passed"
        or summary.get("baseline_valid") is not True
        or summary.get("invalid_reasons") != []
    ):
        raise BaselineError("report source artifact must be valid")

    target_sha = validate_full_sha(
        _require_nonempty_string(provenance.get("target_sha"), "provenance.json target_sha")
    )
    run_id = validate_run_id(
        _require_nonempty_string(provenance.get("run_id"), "provenance.json run_id")
    )
    mode = _require_nonempty_string(provenance.get("mode"), "provenance.json mode")
    if mode not in BENCHMARK_MODES:
        raise BaselineError("benchmark reports require bench-smoke or bench-full artifacts")
    if run_dir.parents[1].name != f"baseline-v{SCHEMA_VERSION}":
        raise BaselineError(f"artifact path must use baseline-v{SCHEMA_VERSION}")
    if run_dir.parent.name != target_sha or run_dir.name != run_id:
        raise BaselineError("artifact path, target SHA, or run ID do not match")
    for field, expected in (
        ("target_sha", target_sha),
        ("run_id", run_id),
        ("mode", mode),
    ):
        if summary.get(field) != expected:
            raise BaselineError(f"summary.json {field} does not match provenance.json")
    started_utc = _require_nonempty_string(
        provenance.get("started_utc"), "provenance.json started_utc"
    )
    ended_utc = _require_nonempty_string(
        provenance.get("ended_utc"), "provenance.json ended_utc"
    )
    recorded_environment = _validate_report_environment(environment)
    criterion_version = verified_criterion_version(repo_root, target_sha)
    stress_scenarios_report, correctness = _report_stress_scenarios(
        repo_root, run_dir, mode, summary
    )
    criterion_scenarios = _report_criterion_scenarios(repo_root, run_dir, summary)
    report = {
        "correctness": correctness,
        "evidence": dict(REPORT_EVIDENCE),
        "producers": _report_producers(
            stress=True,
            criterion=True,
            criterion_version=criterion_version,
        ),
        "report_schema": {
            "name": REPORT_SCHEMA_NAME,
            "version": REPORT_SCHEMA_VERSION,
        },
        "run": {
            "mode": mode,
            "run_id": run_id,
            "source_status": summary["status"],
            "status": "valid",
            "target_sha": target_sha,
        },
        "runner": {
            "environment": recorded_environment,
            "label": recorded_environment["runner_label"],
        },
        "scenarios": stress_scenarios_report + criterion_scenarios,
        "source_artifact": {
            "checksum_inventory": "checksums.sha256",
            "checksum_semantics": "integrity_inventory_not_signature_or_attestation",
            "path": artifact_path,
            "provenance": {
                "ended_utc": ended_utc,
                "harness_path": HARNESS_PATH,
                "harness_version": HARNESS_VERSION,
                "started_utc": started_utc,
            },
            "schema": {"name": "rusty-modbus-baseline-artifact", "version": SCHEMA_VERSION},
        },
    }
    report_errors = validate_report_document(report)
    if report_errors:
        raise BaselineError("generated report is invalid: " + "; ".join(report_errors))
    return report


def _all_numbers_finite(value: Any) -> bool:
    if isinstance(value, bool) or value is None or isinstance(value, str):
        return True
    if isinstance(value, (int, float)):
        try:
            return math.isfinite(float(value))
        except (OverflowError, ValueError):
            return False
    if isinstance(value, list):
        return all(_all_numbers_finite(item) for item in value)
    if isinstance(value, dict):
        return all(isinstance(key, str) and _all_numbers_finite(item) for key, item in value.items())
    return False


def _require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise BaselineError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise BaselineError(
            f"{label} keys must be {sorted(expected)}; "
            f"missing={sorted(expected - actual)}, extra={sorted(actual - expected)}"
        )
    return value


def _validate_report_statistics(
    value: Any, label: str, *, expected_count: int
) -> None:
    statistics_value = _require_exact_keys(
        value,
        {
            "coefficient_of_variation",
            "count",
            "max",
            "mean",
            "median",
            "min",
            "sample_stddev",
        },
        label,
    )
    count = _strict_int(statistics_value["count"], f"{label}.count", minimum=1)
    if count != expected_count:
        raise BaselineError(f"{label}.count must equal scenario repetitions")
    numbers = {
        field: _strict_number(statistics_value[field], f"{label}.{field}")
        for field in ("min", "median", "mean", "max", "sample_stddev")
    }
    coefficient = statistics_value["coefficient_of_variation"]
    if coefficient is not None:
        _strict_number(coefficient, f"{label}.coefficient_of_variation")
    if not numbers["min"] <= numbers["median"] <= numbers["max"]:
        raise BaselineError(f"{label} min/median/max are incoherent")
    if not numbers["min"] <= numbers["mean"] <= numbers["max"]:
        raise BaselineError(f"{label} mean is outside min/max")


def _validate_report_environment_shape(value: Any, runner_label: str) -> None:
    environment = _require_exact_keys(
        value,
        {
            "cargo_metadata",
            "collection_status",
            "cpu",
            "github",
            "platform",
            "power",
            "runner_label",
            "tools",
        },
        "runner.environment",
    )
    if environment["collection_status"] != "complete":
        raise BaselineError("runner.environment.collection_status must be complete")
    if environment["runner_label"] != runner_label:
        raise BaselineError("runner label does not match recorded environment")
    github = environment["github"]
    if not isinstance(github, dict) or any(
        not isinstance(key, str) or not isinstance(item, str)
        for key, item in github.items()
    ):
        raise BaselineError("runner.environment.github must contain string values")
    platform_value = _require_exact_keys(
        environment["platform"],
        {"architecture", "kernel", "os", "release"},
        "runner.environment.platform",
    )
    for field in platform_value:
        _require_nonempty_string(
            platform_value[field], f"runner.environment.platform.{field}"
        )
    cpu = _require_exact_keys(
        environment["cpu"],
        {"logical_count", "model", "model_source"},
        "runner.environment.cpu",
    )
    for field in ("model", "model_source"):
        if cpu[field] is not None and not isinstance(cpu[field], str):
            raise BaselineError(f"runner.environment.cpu.{field} must be a string or null")
    if cpu["logical_count"] is not None:
        _strict_int(cpu["logical_count"], "runner.environment.cpu.logical_count", minimum=1)
    power = _require_exact_keys(
        environment["power"],
        {"availability", "source", "value"},
        "runner.environment.power",
    )
    availability = _require_nonempty_string(
        power["availability"], "runner.environment.power.availability"
    )
    if availability not in {"available", "unavailable"}:
        raise BaselineError("runner.environment.power.availability is unsupported")
    if power["source"] is not None and not isinstance(power["source"], str):
        raise BaselineError("runner.environment.power.source must be a string or null")
    tools = _require_exact_keys(
        environment["tools"], {"cargo", "python", "rustc"}, "runner.environment.tools"
    )
    _require_nonempty_string(tools["cargo"], "runner.environment.tools.cargo")
    rustc = _require_exact_keys(
        tools["rustc"], {"host", "version"}, "runner.environment.tools.rustc"
    )
    for field in rustc:
        _require_nonempty_string(rustc[field], f"runner.environment.tools.rustc.{field}")
    python = _require_exact_keys(
        tools["python"],
        {"executable", "implementation", "version"},
        "runner.environment.tools.python",
    )
    for field in python:
        _require_nonempty_string(python[field], f"runner.environment.tools.python.{field}")
    metadata = _require_exact_keys(
        environment["cargo_metadata"],
        {"packages", "target_directory", "workspace_member_count", "workspace_root"},
        "runner.environment.cargo_metadata",
    )
    _require_nonempty_string(
        metadata["target_directory"], "runner.environment.cargo_metadata.target_directory"
    )
    _require_nonempty_string(
        metadata["workspace_root"], "runner.environment.cargo_metadata.workspace_root"
    )
    _strict_int(
        metadata["workspace_member_count"],
        "runner.environment.cargo_metadata.workspace_member_count",
    )
    if not isinstance(metadata["packages"], list):
        raise BaselineError("runner.environment.cargo_metadata.packages must be a list")


def _criterion_report_producer(report: dict[str, Any]) -> dict[str, Any]:
    producers = report.get("producers")
    if not isinstance(producers, list):
        raise BaselineError("producers must be a list")
    matches = [
        producer
        for producer in producers
        if isinstance(producer, dict) and producer.get("id") == CRITERION_PRODUCER_ID
    ]
    if len(matches) != 1:
        raise BaselineError("report must contain one Criterion producer")
    return matches[0]


def validate_report_document(report: Any) -> list[str]:
    errors: list[str] = []
    if not isinstance(report, dict):
        return ["report root must be an object"]
    try:
        _require_exact_keys(
            report,
            {
                "correctness",
                "evidence",
                "producers",
                "report_schema",
                "run",
                "runner",
                "scenarios",
                "source_artifact",
            },
            "report",
        )
    except BaselineError as error:
        errors.append(str(error))
    report_schema = report.get("report_schema")
    try:
        report_schema = _require_exact_keys(
            report_schema, {"name", "version"}, "report_schema"
        )
        if (
            report_schema["name"] != REPORT_SCHEMA_NAME
            or _strict_int(report_schema["version"], "report_schema.version", minimum=1)
            != REPORT_SCHEMA_VERSION
        ):
            raise BaselineError("report schema is unsupported")
    except BaselineError:
        errors.append(
            f"report_schema must be {REPORT_SCHEMA_NAME} version {REPORT_SCHEMA_VERSION}"
        )
    if report.get("evidence") != REPORT_EVIDENCE:
        errors.append("report evidence classification is missing or unsupported")
    source = report.get("source_artifact")
    if not isinstance(source, dict):
        errors.append("source_artifact must be an object")
    else:
        try:
            _require_exact_keys(
                source,
                {"checksum_inventory", "checksum_semantics", "path", "provenance", "schema"},
                "source_artifact",
            )
        except BaselineError as error:
            errors.append(str(error))
        try:
            source_schema = _require_exact_keys(
                source.get("schema"), {"name", "version"}, "source_artifact.schema"
            )
            if (
                source_schema["name"] != "rusty-modbus-baseline-artifact"
                or _strict_int(
                    source_schema["version"], "source_artifact.schema.version", minimum=1
                )
                != SCHEMA_VERSION
            ):
                raise BaselineError("source artifact schema is unsupported")
        except BaselineError:
            errors.append("source artifact schema is unsupported")
        try:
            source_parts = _relative_parts(source.get("path"), "source_artifact.path")
            if len(source_parts) < 3 or source_parts[-3:] != (
                f"baseline-v{SCHEMA_VERSION}",
                report.get("run", {}).get("target_sha")
                if isinstance(report.get("run"), dict)
                else None,
                report.get("run", {}).get("run_id")
                if isinstance(report.get("run"), dict)
                else None,
            ):
                raise BaselineError("source artifact path does not match report run")
        except BaselineError as error:
            errors.append(str(error))
        if source.get("checksum_inventory") != "checksums.sha256":
            errors.append("source checksum inventory reference must be checksums.sha256")
        if source.get("checksum_semantics") != "integrity_inventory_not_signature_or_attestation":
            errors.append("source checksum semantics are unsupported")
        provenance = source.get("provenance")
        try:
            provenance = _require_exact_keys(
                provenance,
                {"ended_utc", "harness_path", "harness_version", "started_utc"},
                "source_artifact.provenance",
            )
            if (provenance["harness_path"], provenance["harness_version"]) != (
                HARNESS_PATH,
                HARNESS_VERSION,
            ):
                raise BaselineError("source producer provenance is unsupported")
            _require_nonempty_string(
                provenance["started_utc"], "source_artifact.provenance.started_utc"
            )
            _require_nonempty_string(
                provenance["ended_utc"], "source_artifact.provenance.ended_utc"
            )
        except BaselineError as error:
            errors.append(str(error))

    run = report.get("run")
    if not isinstance(run, dict):
        errors.append("run must be an object")
    else:
        try:
            _require_exact_keys(
                run,
                {"mode", "run_id", "source_status", "status", "target_sha"},
                "run",
            )
            validate_full_sha(
                _require_nonempty_string(run.get("target_sha"), "run.target_sha")
            )
            validate_run_id(_require_nonempty_string(run.get("run_id"), "run.run_id"))
        except BaselineError as error:
            errors.append(str(error))
        try:
            run_mode = _require_nonempty_string(run.get("mode"), "run.mode")
        except BaselineError as error:
            errors.append(str(error))
        else:
            if run_mode not in BENCHMARK_MODES:
                errors.append("run mode is unsupported")
        if run.get("status") != "valid" or run.get("source_status") != "passed":
            errors.append("run status must distinguish valid report evidence from passed source")

    runner = report.get("runner")
    try:
        runner = _require_exact_keys(runner, {"environment", "label"}, "runner")
        runner_label = validate_runner_label(
            _require_nonempty_string(runner["label"], "runner.label")
        )
        _validate_report_environment_shape(runner["environment"], runner_label)
    except BaselineError as error:
        errors.append(str(error))

    expected_producers = _report_producers(
        stress=True,
        criterion=True,
        criterion_version=SUPPORTED_CRITERION_VERSION,
    )
    if report.get("producers") != expected_producers:
        errors.append("producer identities or adapter versions are unsupported")

    scenarios = report.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        errors.append("scenarios must be a non-empty list")
        scenarios = []
    stress_count = 0
    criterion_count = 0
    stress_sample_count = 0
    seen_stress_identities: set[tuple[Any, ...]] = set()
    seen_criterion_sources: set[str] = set()
    for index, scenario in enumerate(scenarios):
        if not isinstance(scenario, dict):
            errors.append(f"scenario {index} must be an object")
            continue
        try:
            kind = _require_nonempty_string(
                scenario.get("kind"), f"scenario {index}.kind"
            )
        except BaselineError as error:
            errors.append(str(error))
            continue
        producer_id = scenario.get("producer_id")
        expected_producer = {
            "tcp_stress": STRESS_PRODUCER_ID,
            "criterion_estimate": CRITERION_PRODUCER_ID,
        }.get(kind)
        if expected_producer is None or producer_id != expected_producer:
            errors.append(f"scenario {index} has an unsupported kind or producer")
        else:
            stress_count += kind == "tcp_stress"
            criterion_count += kind == "criterion_estimate"
        if not isinstance(scenario.get("identity"), dict) or not isinstance(
            scenario.get("metrics"), dict
        ):
            errors.append(f"scenario {index} identity and metrics are required")
        sources = scenario.get("sources")
        if not isinstance(sources, list) or not sources or any(
            not isinstance(item, dict) for item in sources
        ):
            errors.append(f"scenario {index} source references are required")
        if kind == "tcp_stress":
            identity = scenario.get("identity")
            metrics = scenario.get("metrics")
            try:
                _require_exact_keys(
                    scenario,
                    {"correctness", "identity", "kind", "metrics", "producer_id", "sources"},
                    f"scenario {index}",
                )
                identity = _require_exact_keys(
                    identity,
                    {
                        "clients",
                        "duration_seconds",
                        "in_flight",
                        "operation",
                        "registers",
                        "repetitions",
                        "transport",
                        "warmup_seconds",
                    },
                    f"scenario {index}.identity",
                )
                transport = _require_nonempty_string(
                    identity["transport"], "identity.transport"
                )
                if transport != "tcp":
                    raise BaselineError("identity must declare TCP")
                operation = _require_nonempty_string(
                    identity["operation"], "identity.operation"
                )
                if operation not in {"read", "mixed"}:
                    raise BaselineError("identity operation is unsupported")
                for field in ("clients", "in_flight", "registers", "repetitions"):
                    _strict_int(identity[field], f"identity.{field}", minimum=1)
                for field in ("duration_seconds", "warmup_seconds"):
                    _strict_int(identity[field], f"identity.{field}")
                repetitions = identity["repetitions"]
                stress_identity = tuple(identity[field] for field in sorted(identity))
                if stress_identity in seen_stress_identities:
                    raise BaselineError("duplicate stress scenario identity")
                seen_stress_identities.add(stress_identity)
                metrics = _require_exact_keys(
                    metrics,
                    {"p99_latency", "throughput"},
                    f"scenario {index}.metrics",
                )
                for field, unit in (
                    ("throughput", "operations_per_second"),
                    ("p99_latency", "milliseconds"),
                ):
                    metric = _require_exact_keys(
                        metrics[field],
                        {"recorded_statistics", "unit"},
                        f"scenario {index}.metrics.{field}",
                    )
                    if metric["unit"] != unit:
                        raise BaselineError(f"{field} metric unit is unsupported")
                    _validate_report_statistics(
                        metric["recorded_statistics"],
                        f"scenario {index}.metrics.{field}.recorded_statistics",
                        expected_count=repetitions,
                    )
                if not isinstance(sources, list) or len(sources) != repetitions:
                    raise BaselineError("stress source count must equal repetitions")
                source_repetitions = []
                for source_reference in sources:
                    source_reference = _require_exact_keys(
                        source_reference,
                        {"command_record", "parsed_sample", "raw_stdout", "repetition"},
                        f"scenario {index} stress source",
                    )
                    repetition = _strict_int(
                        source_reference["repetition"],
                        "stress source repetition",
                        minimum=1,
                    )
                    source_repetitions.append(repetition)
                    command_parts = _relative_parts(
                        source_reference["command_record"], "stress source command_record"
                    )
                    if (
                        len(command_parts) != 3
                        or command_parts[0] != "commands"
                        or command_parts[2] != "command.json"
                        or not COMMAND_ID.fullmatch(command_parts[1])
                    ):
                        raise BaselineError("stress command record reference is malformed")
                    if source_reference["raw_stdout"] != (
                        f"commands/{command_parts[1]}/command.stdout"
                    ):
                        raise BaselineError("stress raw stdout reference is incoherent")
                    expected_parsed = (
                        f"stress/parsed/stress-{identity['operation']}-"
                        f"d{identity['in_flight']}-r{repetition}.json"
                    )
                    if source_reference["parsed_sample"] != expected_parsed:
                        raise BaselineError("stress parsed sample reference is incoherent")
                if sorted(source_repetitions) != list(range(1, repetitions + 1)):
                    raise BaselineError("stress source repetitions are incomplete")
                stress_sample_count += repetitions
            except (AttributeError, BaselineError) as error:
                errors.append(f"scenario {index}: {error}")
            correctness = scenario.get("correctness")
            try:
                correctness = _require_exact_keys(
                    correctness,
                    {
                        "error_rate",
                        "retry_attempts",
                        "total_errors",
                        "zero_error_rate",
                        "zero_errors",
                        "zero_retries",
                    },
                    f"scenario {index}.correctness",
                )
                if (
                    _strict_int(correctness["total_errors"], "total_errors") != 0
                    or _strict_int(correctness["retry_attempts"], "retry_attempts") != 0
                    or _strict_number(correctness["error_rate"], "error_rate") != 0
                    or correctness["zero_errors"] is not True
                    or correctness["zero_retries"] is not True
                    or correctness["zero_error_rate"] is not True
                ):
                    raise BaselineError("strict zero-error facts are required")
            except BaselineError:
                errors.append(f"scenario {index} must record strict zero-error facts")
        elif kind == "criterion_estimate":
            identity = scenario.get("identity")
            metrics = scenario.get("metrics")
            try:
                _require_exact_keys(
                    scenario,
                    {"identity", "kind", "metrics", "producer_id", "sources"},
                    f"scenario {index}",
                )
                identity = _require_exact_keys(
                    identity, {"benchmark_id"}, f"scenario {index}.identity"
                )
                _relative_parts(identity["benchmark_id"], "Criterion benchmark_id")
                metrics = _require_exact_keys(
                    metrics, {"mean_estimate"}, f"scenario {index}.metrics"
                )
                estimate = _require_exact_keys(
                    metrics["mean_estimate"],
                    {
                        "confidence_level",
                        "lower",
                        "point",
                        "standard_error",
                        "unit",
                        "upper",
                    },
                    f"scenario {index}.metrics.mean_estimate",
                )
                if estimate["unit"] != "nanoseconds":
                    raise BaselineError("Criterion metric unit is unsupported")
                confidence_level = _strict_number(
                    estimate["confidence_level"], "Criterion confidence_level"
                )
                lower = _strict_number(estimate["lower"], "Criterion lower bound")
                point = _strict_number(estimate["point"], "Criterion point estimate")
                upper = _strict_number(estimate["upper"], "Criterion upper bound")
                _strict_number(estimate["standard_error"], "Criterion standard_error")
                if not 0 < confidence_level <= 1:
                    raise BaselineError("Criterion confidence level is out of range")
                if not lower <= point <= upper:
                    raise BaselineError("Criterion estimate bounds are incoherent")
                if (
                    not isinstance(sources, list)
                    or len(sources) != 1
                    or not isinstance(sources[0], dict)
                    or set(sources[0]) != {"private_estimates_json"}
                ):
                    raise BaselineError("Criterion source reference is malformed")
                criterion_source_parts = _relative_parts(
                    sources[0]["private_estimates_json"], "Criterion estimates source"
                )
                if (
                    len(criterion_source_parts) < 6
                    or criterion_source_parts[:2] != ("criterion", "raw")
                    or criterion_source_parts[-2:] != ("new", "estimates.json")
                    or "/".join(criterion_source_parts[3:-2]) != identity["benchmark_id"]
                ):
                    raise BaselineError("Criterion private estimates reference is incoherent")
                criterion_source = PurePosixPath(*criterion_source_parts).as_posix()
                if criterion_source in seen_criterion_sources:
                    raise BaselineError("duplicate Criterion source reference")
                seen_criterion_sources.add(criterion_source)
            except (AttributeError, BaselineError) as error:
                errors.append(f"scenario {index}: {error}")

    if stress_count == 0 or criterion_count == 0:
        errors.append("report must contain both TCP stress and Criterion scenarios")
    correctness = report.get("correctness")
    try:
        correctness = _require_exact_keys(
            correctness,
            {
                "stress_sample_count",
                "total_errors",
                "total_retry_attempts",
                "zero_errors",
                "zero_retries",
            },
            "correctness",
        )
        if (
            _strict_int(correctness["stress_sample_count"], "stress_sample_count", minimum=1)
            != stress_sample_count
            or _strict_int(correctness["total_errors"], "total_errors") != 0
            or _strict_int(correctness["total_retry_attempts"], "total_retry_attempts") != 0
            or correctness["zero_errors"] is not True
            or correctness["zero_retries"] is not True
        ):
            raise BaselineError("report correctness facts are incoherent")
    except BaselineError:
        errors.append("report correctness must record strict zero-error and zero-retry facts")
    if not _all_numbers_finite(report):
        errors.append("report contains a non-finite or unsupported value")
    return errors


def _markdown_cell(value: Any) -> str:
    if isinstance(value, float):
        rendered = json.dumps(value, allow_nan=False)
    else:
        rendered = str(value)
    return rendered.replace("\\", "\\\\").replace("|", "\\|").replace("\n", " ")


def render_report_markdown(report: dict[str, Any]) -> str:
    errors = validate_report_document(report)
    if errors:
        raise BaselineError("cannot render invalid report: " + "; ".join(errors))
    run = report["run"]
    evidence = report["evidence"]
    source = report["source_artifact"]
    criterion_producer = _criterion_report_producer(report)
    lines = [
        "# Benchmark artifact report",
        "",
        "This report is observational only. It preserves recorded artifact values; it does not ",
        "establish host isolation, performance comparability, a budget decision, statistical ",
        "significance, or performance acceptance.",
        "",
        "## Evidence status",
        "",
        "| Field | Value |",
        "|---|---|",
        f"| Report schema | `{REPORT_SCHEMA_NAME}` v{REPORT_SCHEMA_VERSION} |",
        f"| Source artifact schema | `rusty-modbus-baseline-artifact` v{SCHEMA_VERSION} |",
        f"| Artifact validity | `{evidence['artifact_validity']}` |",
        f"| Classification | `{evidence['classification']}` |",
        f"| Performance comparability | `{evidence['performance_comparability']}` |",
        f"| Runner isolation | `{evidence['runner_isolation']}` |",
        f"| Budget decision | `{evidence['budget_decision']}` |",
        f"| Statistical significance | `{evidence['statistical_significance']}` |",
        "",
        "## Source run",
        "",
        "| Field | Recorded value |",
        "|---|---|",
        f"| Target SHA | `{run['target_sha']}` |",
        f"| Run ID | `{_markdown_cell(run['run_id'])}` |",
        f"| Mode | `{run['mode']}` |",
        f"| Report status | `{run['status']}` |",
        f"| Source status | `{run['source_status']}` |",
        f"| Runner label | `{_markdown_cell(report['runner']['label'])}` |",
        f"| Source artifact | `{_markdown_cell(source['path'])}` |",
        f"| Source started UTC | `{_markdown_cell(source['provenance']['started_utc'])}` |",
        f"| Source ended UTC | `{_markdown_cell(source['provenance']['ended_utc'])}` |",
        "",
        "## Producer adapters",
        "",
        "| Producer | Version | Adapter |",
        "|---|---|---|",
    ]
    for producer in report["producers"]:
        lines.append(
            f"| {_markdown_cell(producer['producer'])} | {_markdown_cell(producer['version'])} "
            f"| {_markdown_cell(producer['adapter'])} |"
        )

    stress = [item for item in report["scenarios"] if item["kind"] == "tcp_stress"]
    lines.extend(
        [
            "",
            "## TCP stress scenarios",
            "",
            "| Operation | In-flight | Clients | Registers | Repetitions | "
            "Throughput mean (ops/s) | p99 mean (ms) | Errors | Retries |",
            "|---|---:|---:|---:|---:|---:|---:|---:|---:|",
        ]
    )
    for scenario in stress:
        identity = scenario["identity"]
        metrics = scenario["metrics"]
        correctness = scenario["correctness"]
        lines.append(
            f"| {_markdown_cell(identity['operation'])} | {identity['in_flight']} | "
            f"{identity['clients']} | {identity['registers']} | {identity['repetitions']} | "
            f"{_markdown_cell(metrics['throughput']['recorded_statistics']['mean'])} | "
            f"{_markdown_cell(metrics['p99_latency']['recorded_statistics']['mean'])} | "
            f"{correctness['total_errors']} | {correctness['retry_attempts']} |"
        )

    criterion = [
        item for item in report["scenarios"] if item["kind"] == "criterion_estimate"
    ]
    lines.extend(
        [
            "",
            "## Criterion estimates",
            "",
            f"Adapter: Criterion {criterion_producer['version']} "
            f"`{criterion_producer['adapter']}`. This is an exact-version ",
            "private-layout adapter, not a stable upstream data API.",
            "",
            "| Benchmark ID | Confidence level | Mean lower (ns) | Mean point (ns) | "
            "Mean upper (ns) | Standard error (ns) | Source |",
            "|---|---:|---:|---:|---:|---:|---|",
        ]
    )
    for scenario in criterion:
        metric = scenario["metrics"]["mean_estimate"]
        lines.append(
            f"| {_markdown_cell(scenario['identity']['benchmark_id'])} | "
            f"{_markdown_cell(metric['confidence_level'])} | "
            f"{_markdown_cell(metric['lower'])} | {_markdown_cell(metric['point'])} | "
            f"{_markdown_cell(metric['upper'])} | {_markdown_cell(metric['standard_error'])} | "
            f"`{_markdown_cell(scenario['sources'][0]['private_estimates_json'])}` |"
        )

    lines.extend(
        [
            "",
            "## Recorded runner environment",
            "",
            "```json",
            json.dumps(
                report["runner"]["environment"],
                indent=2,
                sort_keys=True,
                ensure_ascii=False,
                allow_nan=False,
            ),
            "```",
            "",
            "## Integrity inventory",
            "",
            f"Source-relative checksum inventory: `{source['checksum_inventory']}`. Checksums are an ",
            "integrity inventory, not a signature or attestation.",
            "",
        ]
    )
    return "\n".join(lines)


def report_json_text(report: dict[str, Any]) -> str:
    errors = validate_report_document(report)
    if errors:
        raise BaselineError("cannot serialize invalid report: " + "; ".join(errors))
    return json.dumps(
        report, indent=2, sort_keys=True, ensure_ascii=False, allow_nan=False
    ) + "\n"


def write_report_pair(directory: Path, report: dict[str, Any]) -> tuple[Path, Path]:
    json_path = directory / REPORT_JSON_NAME
    markdown_path = directory / REPORT_MARKDOWN_NAME
    for path in (json_path, markdown_path):
        if path.exists() or path.is_symlink():
            raise BaselineError(f"report output already exists: {path}")
    json_text = report_json_text(report)
    markdown_text = render_report_markdown(report)
    with json_path.open("x", encoding="utf-8", newline="\n") as handle:
        handle.write(json_text)
    with markdown_path.open("x", encoding="utf-8", newline="\n") as handle:
        handle.write(markdown_text)
    return json_path, markdown_path


def resolve_report_output_dir(
    repo_root: Path, run_dir: Path, value: str
) -> Path:
    repo_root = repo_root.resolve()
    raw = Path(value)
    if ".." in raw.parts:
        raise BaselineError("report output directory must not contain path traversal")
    candidate = raw if raw.is_absolute() else repo_root / raw
    try:
        candidate.relative_to(repo_root)
    except ValueError as error:
        raise BaselineError("report output directory must be inside the repository") from error
    _reject_symlink_components(candidate, repo_root, "report output directory")
    resolved = candidate.resolve()
    try:
        relative = resolved.relative_to(repo_root)
    except ValueError as error:
        raise BaselineError("report output directory must be inside the repository") from error
    if not relative.parts:
        raise BaselineError("report output directory must not be the repository root")
    run_dir = run_dir.resolve()
    if resolved == run_dir or run_dir in resolved.parents:
        raise BaselineError("read-only report output must not mutate the source artifact")
    if candidate.exists() or candidate.is_symlink():
        raise BaselineError(f"report output directory already exists: {candidate}")
    return resolved


def render_report_to_directory(
    repo_root: Path, run_dir: Path, output_dir: str
) -> tuple[Path, Path]:
    report = build_benchmark_report(repo_root, run_dir)
    destination = resolve_report_output_dir(repo_root, run_dir, output_dir)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.mkdir()
    return write_report_pair(destination, report)


def load_report_file(repo_root: Path, value: str) -> dict[str, Any]:
    repo_root = repo_root.resolve()
    raw = Path(value)
    if ".." in raw.parts:
        raise BaselineError("report path must not contain path traversal")
    path = raw if raw.is_absolute() else repo_root / raw
    try:
        path.relative_to(repo_root)
    except ValueError as error:
        raise BaselineError("report path must be inside the repository") from error
    _reject_symlink_components(path, repo_root, "report path")
    if not path.is_file():
        raise BaselineError(f"report file does not exist: {path}")
    report = _read_json_object(path, "benchmark report")
    errors = validate_report_document(report)
    if errors:
        raise BaselineError("report validation failed: " + "; ".join(errors))
    criterion_producer = _criterion_report_producer(report)
    verified_criterion_version(
        repo_root,
        report["run"]["target_sha"],
        declared_version=criterion_producer["version"],
    )
    return report


def _comparison_identity_key(
    kind_value: Any, producer_value: Any, identity: Any, label: str
) -> tuple[str | int, ...]:
    kind = _require_nonempty_string(kind_value, f"{label}.kind")
    producer_id = _require_nonempty_string(
        producer_value, f"{label}.producer_id"
    )
    if kind == "tcp_stress":
        if producer_id != STRESS_PRODUCER_ID:
            raise BaselineError(f"{label} has an unsupported TCP stress producer")
        identity = _require_exact_keys(
            identity,
            {
                "clients",
                "duration_seconds",
                "in_flight",
                "operation",
                "registers",
                "repetitions",
                "transport",
                "warmup_seconds",
            },
            f"{label}.identity",
        )
        transport = _require_nonempty_string(
            identity["transport"], f"{label}.identity.transport"
        )
        operation = _require_nonempty_string(
            identity["operation"], f"{label}.identity.operation"
        )
        if transport != "tcp" or operation not in {"read", "mixed"}:
            raise BaselineError(f"{label} has an unsupported TCP stress identity")
        return (
            kind,
            producer_id,
            transport,
            operation,
            _strict_int(
                identity["in_flight"], f"{label}.identity.in_flight", minimum=1
            ),
            _strict_int(identity["clients"], f"{label}.identity.clients", minimum=1),
            _strict_int(
                identity["registers"], f"{label}.identity.registers", minimum=1
            ),
            _strict_int(
                identity["repetitions"], f"{label}.identity.repetitions", minimum=1
            ),
            _strict_int(
                identity["duration_seconds"], f"{label}.identity.duration_seconds"
            ),
            _strict_int(
                identity["warmup_seconds"], f"{label}.identity.warmup_seconds"
            ),
        )
    if kind == "criterion_estimate":
        if producer_id != CRITERION_PRODUCER_ID:
            raise BaselineError(f"{label} has an unsupported Criterion producer")
        identity = _require_exact_keys(
            identity, {"benchmark_id"}, f"{label}.identity"
        )
        benchmark_id = _require_nonempty_string(
            identity["benchmark_id"], f"{label}.identity.benchmark_id"
        )
        _relative_parts(benchmark_id, f"{label}.identity.benchmark_id")
        return (kind, producer_id, benchmark_id)
    raise BaselineError(f"{label}.kind is unsupported")


def _report_scenario_comparison_key(
    scenario: Any, label: str
) -> tuple[str | int, ...]:
    if not isinstance(scenario, dict):
        raise BaselineError(f"{label} must be an object")
    return _comparison_identity_key(
        scenario.get("kind"),
        scenario.get("producer_id"),
        scenario.get("identity"),
        label,
    )


def _comparison_scenario_index(
    report: dict[str, Any], label: str
) -> dict[tuple[str | int, ...], dict[str, Any]]:
    index: dict[tuple[str | int, ...], dict[str, Any]] = {}
    for position, scenario in enumerate(report["scenarios"]):
        key = _report_scenario_comparison_key(
            scenario, f"{label} report scenario {position}"
        )
        if key in index:
            raise BaselineError(
                f"duplicate {key[0]} comparison key in {label} report: {key}"
            )
        index[key] = scenario
    return index


def _observed_delta(
    baseline_value: Any, candidate_value: Any, *, unit: str, label: str
) -> dict[str, float | str]:
    baseline_number = _strict_number(baseline_value, f"baseline {label}")
    candidate_number = _strict_number(candidate_value, f"candidate {label}")
    difference = candidate_number - baseline_number
    if not math.isfinite(difference):
        raise BaselineError(f"{label} candidate-minus-baseline delta must be finite")
    return {
        "baseline": baseline_number,
        "candidate": candidate_number,
        "candidate_minus_baseline": difference,
        "unit": unit,
    }


def _paired_report_metric(
    baseline_metric: Any,
    candidate_metric: Any,
    *,
    expected_keys: set[str],
    expected_unit: str,
    label: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    baseline_metric = _require_exact_keys(
        baseline_metric, expected_keys, f"baseline {label}"
    )
    candidate_metric = _require_exact_keys(
        candidate_metric, expected_keys, f"candidate {label}"
    )
    if baseline_metric["unit"] != candidate_metric["unit"]:
        raise BaselineError(f"{label} units do not match")
    if baseline_metric["unit"] != expected_unit:
        raise BaselineError(f"{label} unit is unsupported")
    return baseline_metric, candidate_metric


def _matched_scenario_observation(
    baseline_scenario: dict[str, Any],
    candidate_scenario: dict[str, Any],
    key: tuple[str | int, ...],
) -> dict[str, Any]:
    kind = key[0]
    baseline_metrics = baseline_scenario["metrics"]
    candidate_metrics = candidate_scenario["metrics"]
    if kind == "tcp_stress":
        metric_names = {"p99_latency", "throughput"}
        baseline_metrics = _require_exact_keys(
            baseline_metrics, metric_names, "baseline TCP stress metrics"
        )
        candidate_metrics = _require_exact_keys(
            candidate_metrics, metric_names, "candidate TCP stress metrics"
        )
        observations = {}
        for metric_name, report_unit, comparison_unit in (
            ("p99_latency", "milliseconds", "ms"),
            ("throughput", "operations_per_second", "operations_per_second"),
        ):
            baseline_metric, candidate_metric = _paired_report_metric(
                baseline_metrics[metric_name],
                candidate_metrics[metric_name],
                expected_keys={"recorded_statistics", "unit"},
                expected_unit=report_unit,
                label=f"{metric_name} metric",
            )
            statistic_keys = {
                "coefficient_of_variation",
                "count",
                "max",
                "mean",
                "median",
                "min",
                "sample_stddev",
            }
            baseline_statistics = _require_exact_keys(
                baseline_metric["recorded_statistics"],
                statistic_keys,
                f"baseline {metric_name} statistics",
            )
            candidate_statistics = _require_exact_keys(
                candidate_metric["recorded_statistics"],
                statistic_keys,
                f"candidate {metric_name} statistics",
            )
            observations[metric_name] = _observed_delta(
                baseline_statistics["mean"],
                candidate_statistics["mean"],
                unit=comparison_unit,
                label=f"{metric_name} mean",
            )
        return {
            "identity": dict(baseline_scenario["identity"]),
            "kind": kind,
            "observations": observations,
            "producer_id": baseline_scenario["producer_id"],
        }

    if kind == "criterion_estimate":
        baseline_metrics = _require_exact_keys(
            baseline_metrics, {"mean_estimate"}, "baseline Criterion metrics"
        )
        candidate_metrics = _require_exact_keys(
            candidate_metrics, {"mean_estimate"}, "candidate Criterion metrics"
        )
        estimate_keys = {
            "confidence_level",
            "lower",
            "point",
            "standard_error",
            "unit",
            "upper",
        }
        baseline_estimate, candidate_estimate = _paired_report_metric(
            baseline_metrics["mean_estimate"],
            candidate_metrics["mean_estimate"],
            expected_keys=estimate_keys,
            expected_unit="nanoseconds",
            label="Criterion mean estimate",
        )
        if baseline_estimate["confidence_level"] != candidate_estimate["confidence_level"]:
            raise BaselineError("Criterion confidence levels do not match")
        return {
            "identity": dict(baseline_scenario["identity"]),
            "kind": kind,
            "observations": {
                "mean_estimate": _observed_delta(
                    baseline_estimate["point"],
                    candidate_estimate["point"],
                    unit="ns",
                    label="Criterion mean point estimate",
                )
            },
            "producer_id": baseline_scenario["producer_id"],
        }
    raise BaselineError(f"comparison scenario kind is unsupported: {kind!r}")


def _comparison_operand(report: dict[str, Any]) -> dict[str, Any]:
    return {
        "producers": report["producers"],
        "run": report["run"],
        "runner": report["runner"],
        "source_artifact": report["source_artifact"],
    }


def build_benchmark_comparison(
    baseline_report: Any, candidate_report: Any
) -> dict[str, Any]:
    for label, report in (("baseline", baseline_report), ("candidate", candidate_report)):
        errors = validate_report_document(report)
        if errors:
            raise BaselineError(f"{label} report validation failed: " + "; ".join(errors))
    expected_schema = {"name": REPORT_SCHEMA_NAME, "version": REPORT_SCHEMA_VERSION}
    if baseline_report["report_schema"] != candidate_report["report_schema"]:
        raise BaselineError("input report schemas do not match")
    if baseline_report["report_schema"] != expected_schema:
        raise BaselineError("input report schema is unsupported")
    if baseline_report["producers"] != candidate_report["producers"]:
        raise BaselineError("input report producer records do not match")
    if baseline_report["run"]["mode"] != candidate_report["run"]["mode"]:
        raise BaselineError("input report run modes do not match")

    baseline_index = _comparison_scenario_index(baseline_report, "baseline")
    candidate_index = _comparison_scenario_index(candidate_report, "candidate")
    baseline_keys = set(baseline_index)
    candidate_keys = set(candidate_index)
    if baseline_keys != candidate_keys:
        raise BaselineError(
            "comparison scenario sets do not match; "
            f"missing_from_candidate={sorted(baseline_keys - candidate_keys)}, "
            f"extra_in_candidate={sorted(candidate_keys - baseline_keys)}"
        )

    comparison = {
        "comparison_schema": {
            "name": COMPARISON_SCHEMA_NAME,
            "version": COMPARISON_SCHEMA_VERSION,
        },
        "evidence": dict(COMPARISON_EVIDENCE),
        "input_report_schema": dict(expected_schema),
        "operands": {
            "baseline": _comparison_operand(baseline_report),
            "candidate": _comparison_operand(candidate_report),
        },
        "scenarios": [
            _matched_scenario_observation(
                baseline_index[key], candidate_index[key], key
            )
            for key in sorted(baseline_keys)
        ],
    }
    errors = validate_comparison_document(comparison)
    if errors:
        raise BaselineError("generated comparison is invalid: " + "; ".join(errors))
    return comparison


def _validate_comparison_operand(value: Any, label: str) -> dict[str, Any]:
    operand = _require_exact_keys(
        value, {"producers", "run", "runner", "source_artifact"}, label
    )
    expected_producers = _report_producers(
        stress=True,
        criterion=True,
        criterion_version=SUPPORTED_CRITERION_VERSION,
    )
    if operand["producers"] != expected_producers:
        raise BaselineError(f"{label}.producers are unsupported")
    run = _require_exact_keys(
        operand["run"],
        {"mode", "run_id", "source_status", "status", "target_sha"},
        f"{label}.run",
    )
    target_sha = validate_full_sha(
        _require_nonempty_string(run["target_sha"], f"{label}.run.target_sha")
    )
    run_id = validate_run_id(
        _require_nonempty_string(run["run_id"], f"{label}.run.run_id")
    )
    mode = _require_nonempty_string(run["mode"], f"{label}.run.mode")
    if mode not in BENCHMARK_MODES:
        raise BaselineError(f"{label}.run.mode is unsupported")
    if run["status"] != "valid" or run["source_status"] != "passed":
        raise BaselineError(f"{label}.run statuses are unsupported")

    runner = _require_exact_keys(
        operand["runner"], {"environment", "label"}, f"{label}.runner"
    )
    runner_label = validate_runner_label(
        _require_nonempty_string(runner["label"], f"{label}.runner.label")
    )
    _validate_report_environment_shape(runner["environment"], runner_label)

    source = _require_exact_keys(
        operand["source_artifact"],
        {"checksum_inventory", "checksum_semantics", "path", "provenance", "schema"},
        f"{label}.source_artifact",
    )
    source_schema = _require_exact_keys(
        source["schema"], {"name", "version"}, f"{label}.source_artifact.schema"
    )
    if (
        source_schema["name"] != "rusty-modbus-baseline-artifact"
        or _strict_int(
            source_schema["version"],
            f"{label}.source_artifact.schema.version",
            minimum=1,
        )
        != SCHEMA_VERSION
    ):
        raise BaselineError(f"{label}.source_artifact.schema is unsupported")
    source_parts = _relative_parts(source["path"], f"{label}.source_artifact.path")
    if len(source_parts) < 3 or source_parts[-3:] != (
        f"baseline-v{SCHEMA_VERSION}",
        target_sha,
        run_id,
    ):
        raise BaselineError(f"{label}.source_artifact.path does not match its run")
    if source["checksum_inventory"] != "checksums.sha256":
        raise BaselineError(f"{label}.source_artifact checksum reference is unsupported")
    if source["checksum_semantics"] != "integrity_inventory_not_signature_or_attestation":
        raise BaselineError(f"{label}.source_artifact checksum semantics are unsupported")
    provenance = _require_exact_keys(
        source["provenance"],
        {"ended_utc", "harness_path", "harness_version", "started_utc"},
        f"{label}.source_artifact.provenance",
    )
    if (provenance["harness_path"], provenance["harness_version"]) != (
        HARNESS_PATH,
        HARNESS_VERSION,
    ):
        raise BaselineError(f"{label}.source_artifact provenance is unsupported")
    for field in ("started_utc", "ended_utc"):
        _require_nonempty_string(
            provenance[field], f"{label}.source_artifact.provenance.{field}"
        )
    return operand


def _strict_signed_number(value: Any, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise BaselineError(f"{label} must be numeric")
    try:
        result = float(value)
    except (OverflowError, ValueError) as error:
        raise BaselineError(f"{label} must be finite") from error
    if not math.isfinite(result):
        raise BaselineError(f"{label} must be finite")
    return result


def _validate_comparison_observation(value: Any, unit: str, label: str) -> None:
    observation = _require_exact_keys(
        value,
        {"baseline", "candidate", "candidate_minus_baseline", "unit"},
        label,
    )
    if observation["unit"] != unit:
        raise BaselineError(f"{label}.unit must be {unit}")
    baseline_value = _strict_number(observation["baseline"], f"{label}.baseline")
    candidate_value = _strict_number(observation["candidate"], f"{label}.candidate")
    difference = _strict_signed_number(
        observation["candidate_minus_baseline"],
        f"{label}.candidate_minus_baseline",
    )
    expected = candidate_value - baseline_value
    if not math.isfinite(expected) or difference != expected:
        raise BaselineError(f"{label}.candidate_minus_baseline is not the observed delta")


def _validate_comparison_scenario(
    value: Any, position: int
) -> tuple[str | int, ...]:
    label = f"comparison scenario {position}"
    scenario = _require_exact_keys(
        value, {"identity", "kind", "observations", "producer_id"}, label
    )
    key = _comparison_identity_key(
        scenario["kind"], scenario["producer_id"], scenario["identity"], label
    )
    kind = key[0]
    observations = scenario["observations"]
    if kind == "tcp_stress":
        observations = _require_exact_keys(
            observations, {"p99_latency", "throughput"}, f"{label}.observations"
        )
        _validate_comparison_observation(
            observations["p99_latency"], "ms", f"{label}.observations.p99_latency"
        )
        _validate_comparison_observation(
            observations["throughput"],
            "operations_per_second",
            f"{label}.observations.throughput",
        )
        return key
    if kind == "criterion_estimate":
        observations = _require_exact_keys(
            observations, {"mean_estimate"}, f"{label}.observations"
        )
        _validate_comparison_observation(
            observations["mean_estimate"],
            "ns",
            f"{label}.observations.mean_estimate",
        )
        return key
    raise BaselineError(f"{label}.kind is unsupported")


def validate_comparison_document(comparison: Any) -> list[str]:
    if not isinstance(comparison, dict):
        return ["comparison root must be an object"]
    try:
        comparison = _require_exact_keys(
            comparison,
            {
                "comparison_schema",
                "evidence",
                "input_report_schema",
                "operands",
                "scenarios",
            },
            "comparison",
        )
        comparison_schema = _require_exact_keys(
            comparison["comparison_schema"],
            {"name", "version"},
            "comparison_schema",
        )
        if (
            comparison_schema["name"] != COMPARISON_SCHEMA_NAME
            or _strict_int(
                comparison_schema["version"], "comparison_schema.version", minimum=1
            )
            != COMPARISON_SCHEMA_VERSION
        ):
            raise BaselineError("comparison schema is unsupported")
        input_schema = _require_exact_keys(
            comparison["input_report_schema"],
            {"name", "version"},
            "input_report_schema",
        )
        if (
            input_schema["name"] != REPORT_SCHEMA_NAME
            or _strict_int(
                input_schema["version"], "input_report_schema.version", minimum=1
            )
            != REPORT_SCHEMA_VERSION
        ):
            raise BaselineError("input report schema is unsupported")
        if comparison["evidence"] != COMPARISON_EVIDENCE:
            raise BaselineError("comparison evidence semantics are unsupported")

        operands = _require_exact_keys(
            comparison["operands"], {"baseline", "candidate"}, "operands"
        )
        baseline_operand = _validate_comparison_operand(
            operands["baseline"], "operands.baseline"
        )
        candidate_operand = _validate_comparison_operand(
            operands["candidate"], "operands.candidate"
        )
        if baseline_operand["producers"] != candidate_operand["producers"]:
            raise BaselineError("operand producer records do not match")
        if baseline_operand["run"]["mode"] != candidate_operand["run"]["mode"]:
            raise BaselineError("operand run modes do not match")

        scenarios = comparison["scenarios"]
        if not isinstance(scenarios, list) or not scenarios:
            raise BaselineError("comparison scenarios must be a non-empty list")
        keys = [
            _validate_comparison_scenario(scenario, position)
            for position, scenario in enumerate(scenarios)
        ]
        if len(set(keys)) != len(keys):
            raise BaselineError("comparison scenarios contain duplicate keys")
        if keys != sorted(keys):
            raise BaselineError("comparison scenarios are not in canonical key order")
        if {key[0] for key in keys} != {"criterion_estimate", "tcp_stress"}:
            raise BaselineError("comparison must contain TCP stress and Criterion scenarios")
        if not _all_numbers_finite(comparison):
            raise BaselineError("comparison contains a non-finite or unsupported value")
    except BaselineError as error:
        return [str(error)]
    return []


def comparison_json_text(comparison: dict[str, Any]) -> str:
    errors = validate_comparison_document(comparison)
    if errors:
        raise BaselineError("cannot serialize invalid comparison: " + "; ".join(errors))
    return json.dumps(
        comparison, indent=2, sort_keys=True, ensure_ascii=False, allow_nan=False
    ) + "\n"


def compare_report_files(
    repo_root: Path, baseline_report_file: str, candidate_report_file: str
) -> dict[str, Any]:
    baseline_report = load_report_file(repo_root, baseline_report_file)
    candidate_report = load_report_file(repo_root, candidate_report_file)
    return build_benchmark_comparison(baseline_report, candidate_report)


def run_correctness(run: ArtifactRun) -> None:
    for spec in correctness_plan(run.repo_root):
        run.run_command(spec)


def _stress_binary(target_directory: Path) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    return target_directory / "release" / f"stress-test{suffix}"


def run_benchmarks(
    run: ArtifactRun,
    *,
    mode: str,
    duration: int,
    warmup: int,
    repetitions: int,
    benchmark_targets: Sequence[str],
    target_directory: Path,
) -> None:
    run.run_command(
        CommandSpec(
            "build-stress",
            (
                "cargo",
                "build",
                "--release",
                "-p",
                "rusty-modbus-benchmarks",
                "--bin",
                "stress-test",
                "--locked",
            ),
            run.repo_root,
        )
    )
    binary = _stress_binary(target_directory)
    if not binary.is_file():
        raise BaselineError(f"stress binary is missing after build: {binary}")
    scenarios = stress_scenarios(mode, repetitions)
    seen: set[tuple[Any, ...]] = set()
    stress_dir = run.run_dir / "stress" / "parsed"
    stress_dir.mkdir(parents=True)
    for scenario in scenarios:
        key = (
            scenario["transport"],
            scenario["operation"],
            scenario["in_flight"],
            scenario["clients"],
            scenario["registers"],
            scenario["repetition"],
        )
        if key in seen:
            raise BaselineError(f"duplicate planned stress scenario: {key}")
        seen.add(key)
        label = (
            f"stress-{scenario['operation']}-d{scenario['in_flight']}-"
            f"r{scenario['repetition']}"
        )
        result = run.run_command(
            CommandSpec(
                label,
                (
                    str(binary),
                    "--transport",
                    scenario["transport"],
                    "--operation",
                    scenario["operation"],
                    "--in-flight",
                    str(scenario["in_flight"]),
                    "--clients",
                    str(scenario["clients"]),
                    "--registers",
                    str(scenario["registers"]),
                    "--duration",
                    str(duration),
                    "--warmup",
                    str(warmup),
                    "--json",
                ),
                run.repo_root,
            ),
            timeout=float(duration + warmup + 30),
        )
        expected = {
            **scenario,
            "duration_secs": duration,
            "warmup_secs": warmup,
        }
        parsed = parse_stress_json(result.stdout, expected)
        parsed["repetition"] = scenario["repetition"]
        parsed["command_id"] = result.command_id
        run.stress_samples.append(parsed)
        write_json(stress_dir / f"{label}.json", parsed)
    run.stress_aggregates = aggregate_stress_samples(run.stress_samples, scenarios)

    criterion_results = []
    for spec in benchmark_criterion_specs(mode, run.repo_root, benchmark_targets, run.run_dir):
        criterion_home = Path(dict(spec.env)["CRITERION_HOME"])
        criterion_home.parent.mkdir(parents=True, exist_ok=True)
        run.run_command(spec)
        criterion_results.extend(parse_criterion_estimates(criterion_home, run.repo_root))
    if not criterion_results:
        raise BaselineError("Criterion produced no parsed estimates")
    run.criterion_results = sorted(
        criterion_results, key=lambda item: item["source"].encode("utf-8")
    )
    write_json(run.run_dir / "criterion" / "parsed-estimates.json", run.criterion_results)


def _bounded_int(value: str, *, minimum: int, maximum: int, label: str) -> int:
    try:
        result = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError(f"{label} must be an integer") from error
    if not minimum <= result <= maximum:
        raise argparse.ArgumentTypeError(f"{label} must be between {minimum} and {maximum}")
    return result


def duration_arg(value: str) -> int:
    return _bounded_int(value, minimum=1, maximum=300, label="duration")


def warmup_arg(value: str) -> int:
    return _bounded_int(value, minimum=0, maximum=300, label="warmup")


def repetitions_arg(value: str) -> int:
    return _bounded_int(value, minimum=1, maximum=20, label="repetitions")


def add_common_run_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--output-root", default=DEFAULT_OUTPUT_ROOT)
    parser.add_argument("--run-id")
    parser.add_argument("--runner-label", required=True)
    parser.add_argument("--allow-dirty", action="store_true")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    correctness = subparsers.add_parser("correctness")
    add_common_run_arguments(correctness)
    smoke = subparsers.add_parser("bench-smoke")
    add_common_run_arguments(smoke)
    smoke.add_argument("--duration", type=duration_arg, default=1)
    smoke.add_argument("--warmup", type=warmup_arg, default=1)
    smoke.add_argument("--repetitions", type=repetitions_arg, default=1)
    full = subparsers.add_parser("bench-full")
    add_common_run_arguments(full)
    full.add_argument("--duration", type=duration_arg, default=5)
    full.add_argument("--warmup", type=warmup_arg, default=1)
    full.add_argument("--repetitions", type=repetitions_arg, default=5)
    validate = subparsers.add_parser(
        "validate", help="validate a retained baseline artifact and checksum inventory"
    )
    validate.add_argument("run_dir")
    report = subparsers.add_parser(
        "report",
        help="validate a benchmark artifact and render observational JSON and Markdown",
    )
    report.add_argument("run_dir")
    report.add_argument(
        "--output-dir",
        required=True,
        help="new repository-local directory; never overwrites or mutates the source artifact",
    )
    validate_report = subparsers.add_parser(
        "validate-report", help="validate a benchmark-report JSON document"
    )
    validate_report.add_argument("report_json")
    compare_report = subparsers.add_parser(
        "compare-report",
        help="emit signed observational deltas for two validated benchmark reports",
    )
    compare_report.add_argument("baseline_report_json")
    compare_report.add_argument("candidate_report_json")
    return parser


def run_mode(args: argparse.Namespace, repo_root: Path) -> tuple[int, Path | None]:
    repository, bootstrap = bootstrap_repository(repo_root)
    enforce_clean_worktree(repository["dirty"], args.allow_dirty)
    output_root = resolve_output_root(repo_root, args.output_root)
    run = ArtifactRun(
        repo_root=repo_root,
        output_root=output_root,
        target_sha=repository["target_sha"],
        run_id=args.run_id or default_run_id(),
        mode=args.command,
        runner_label=args.runner_label,
        dirty=repository["dirty"],
        allow_dirty=args.allow_dirty,
    )
    run.create()
    run.provenance.update(repository)
    for label, result in zip(("git-head", "git-status", "git-branch"), bootstrap, strict=True):
        run.record_bootstrap(label, result)
    failure: BaseException | None = None
    try:
        benchmark_targets, target_directory = collect_environment(run)
        if args.command == "correctness":
            run_correctness(run)
        else:
            run_benchmarks(
                run,
                mode=args.command,
                duration=args.duration,
                warmup=args.warmup,
                repetitions=args.repetitions,
                benchmark_targets=benchmark_targets,
                target_directory=target_directory,
            )
    except BaseException as caught:
        failure = caught
        run.add_error(caught)
    finally:
        try:
            run.finalize()
        except BaseException as finalize_error:
            if failure is None:
                failure = finalize_error
            print(f"baseline finalization failed: {finalize_error}", file=sys.stderr)
    print(f"artifact: {run._repo_relative(run.run_dir)}")
    if failure is not None:
        print(f"baseline failed: {failure}", file=sys.stderr)
        return 1, run.run_dir
    return 0, run.run_dir


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    repo_root = Path(__file__).resolve().parent.parent
    try:
        if args.command == "validate":
            run_dir = Path(args.run_dir)
            if not run_dir.is_absolute():
                run_dir = repo_root / run_dir
            errors = validate_artifact(repo_root, run_dir)
            if errors:
                for error in errors:
                    print(f"baseline artifact: {error}", file=sys.stderr)
                return 1
            print(f"baseline artifact valid: {run_dir.resolve().relative_to(repo_root)}")
            return 0
        if args.command == "report":
            run_dir = Path(args.run_dir)
            if not run_dir.is_absolute():
                run_dir = repo_root / run_dir
            json_path, markdown_path = render_report_to_directory(
                repo_root, run_dir, args.output_dir
            )
            print(f"benchmark report JSON: {json_path.relative_to(repo_root)}")
            print(f"benchmark report Markdown: {markdown_path.relative_to(repo_root)}")
            return 0
        if args.command == "validate-report":
            load_report_file(repo_root, args.report_json)
            print(f"benchmark report valid: {args.report_json}")
            return 0
        if args.command == "compare-report":
            comparison = compare_report_files(
                repo_root,
                args.baseline_report_json,
                args.candidate_report_json,
            )
            sys.stdout.write(comparison_json_text(comparison))
            return 0
        status, _ = run_mode(args, repo_root)
        return status
    except BaselineError as error:
        print(f"baseline: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
