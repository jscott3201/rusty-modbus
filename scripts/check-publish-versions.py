#!/usr/bin/env python3
"""Reject mixed release metadata before package publication."""

from __future__ import annotations

import json
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]


class CheckError(Exception):
    """Raised when release metadata cannot be inspected."""


@dataclass
class RepositoryData:
    root_manifest: dict[str, Any]
    workspace_manifests: tuple[tuple[str, dict[str, Any]], ...]
    python_manifest: dict[str, Any]
    pyproject: dict[str, Any]
    fuzz_manifest: dict[str, Any]
    locks: dict[str, dict[str, Any]]


@dataclass(frozen=True)
class ValidationResult:
    errors: tuple[str, ...]
    candidate: str
    package_count: int
    edge_count: int
    lock_counts: tuple[tuple[str, int], ...]


def _load_toml(path: Path) -> dict[str, Any]:
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise CheckError(f"cannot read {path}: {error}") from error


def _workspace_manifests(
    root: Path, root_manifest: dict[str, Any]
) -> tuple[tuple[str, dict[str, Any]], ...]:
    workspace = root_manifest.get("workspace")
    if not isinstance(workspace, dict) or not isinstance(workspace.get("members"), list):
        raise CheckError("Cargo.toml has no [workspace].members list")

    excluded = {
        (root / entry).resolve()
        for entry in workspace.get("exclude", [])
        if isinstance(entry, str)
    }
    paths: set[Path] = set()
    for pattern in workspace["members"]:
        if not isinstance(pattern, str):
            raise CheckError("Cargo.toml contains a non-string workspace member")
        for match in root.glob(pattern):
            if not match.is_dir():
                continue
            directory = match
            if directory.resolve() not in excluded:
                paths.add(directory / "Cargo.toml")

    manifests = []
    for path in sorted(paths):
        if not path.is_file():
            raise CheckError(f"workspace package manifest is missing: {path}")
        manifests.append((path.relative_to(root).as_posix(), _load_toml(path)))
    return tuple(manifests)


def load_repository(root: Path = ROOT) -> RepositoryData:
    root_manifest = _load_toml(root / "Cargo.toml")
    return RepositoryData(
        root_manifest=root_manifest,
        workspace_manifests=_workspace_manifests(root, root_manifest),
        python_manifest=_load_toml(root / "crates/rusty-modbus-python/Cargo.toml"),
        pyproject=_load_toml(root / "crates/rusty-modbus-python/pyproject.toml"),
        fuzz_manifest=_load_toml(root / "fuzz/Cargo.toml"),
        locks={
            "Cargo.lock": _load_toml(root / "Cargo.lock"),
            "crates/rusty-modbus-python/Cargo.lock": _load_toml(
                root / "crates/rusty-modbus-python/Cargo.lock"
            ),
            "fuzz/Cargo.lock": _load_toml(root / "fuzz/Cargo.lock"),
        },
    )


def cargo_metadata(root: Path = ROOT) -> dict[str, Any]:
    command = [
        "cargo",
        "metadata",
        "--locked",
        "--no-deps",
        "--format-version",
        "1",
    ]
    try:
        result = subprocess.run(
            command,
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        )
        return json.loads(result.stdout)
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or error.stdout.strip() or f"exit {error.returncode}"
        raise CheckError(f"cargo metadata --locked failed:\n{detail}") from error
    except json.JSONDecodeError as error:
        raise CheckError(f"cargo metadata returned invalid JSON: {error}") from error


def _candidate(data: RepositoryData) -> str | None:
    workspace = data.root_manifest.get("workspace")
    package = workspace.get("package") if isinstance(workspace, dict) else None
    version = package.get("version") if isinstance(package, dict) else None
    return version if isinstance(version, str) and version else None


def _package_inventory(
    data: RepositoryData,
) -> tuple[dict[str, tuple[str, dict[str, Any]]], list[str]]:
    packages: dict[str, tuple[str, dict[str, Any]]] = {}
    errors: list[str] = []
    for path, manifest in data.workspace_manifests:
        package = manifest.get("package")
        if not isinstance(package, dict):
            errors.append(f"{path}: missing [package] table")
            continue
        name = package.get("name")
        if not isinstance(name, str) or not name:
            errors.append(f"{path}: missing [package].name")
            continue
        if name in packages:
            errors.append(f"workspace package name {name!r} is declared more than once")
            continue
        packages[name] = (path, package)
    return packages, errors


def _version_inherits_workspace(value: Any) -> bool:
    return isinstance(value, dict) and value.get("workspace") is True


def _lock_errors(
    label: str,
    lock: dict[str, Any],
    expected_versions: dict[str, str],
    required_names: set[str],
) -> tuple[list[str], int]:
    records = lock.get("package")
    if not isinstance(records, list):
        return [f"{label}: missing package records"], 0

    errors: list[str] = []
    local_records: dict[str, str] = {}
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            errors.append(f"{label}: package record {index} is not a table")
            continue
        if record.get("source") is not None:
            continue
        name = record.get("name")
        version = record.get("version")
        if not isinstance(name, str) or not isinstance(version, str):
            errors.append(f"{label}: local package record {index} has no name or version")
            continue
        if name not in expected_versions:
            errors.append(f"{label}: source-less package {name!r} is not a known local package")
            continue
        if name in local_records:
            errors.append(f"{label}: local package {name} is recorded more than once")
            continue
        local_records[name] = version
        expected = expected_versions[name]
        if version != expected:
            errors.append(
                f"{label}: local package {name} has version {version}; expected {expected}"
            )

    for name in sorted(required_names - local_records.keys()):
        errors.append(f"{label}: missing local package record for {name}")
    return errors, len(local_records)


def validate(
    data: RepositoryData, metadata: dict[str, Any] | None = None
) -> ValidationResult:
    errors: list[str] = []
    candidate = _candidate(data)
    if candidate is None:
        return ValidationResult(
            errors=("Cargo.toml: missing [workspace.package].version",),
            candidate="<missing>",
            package_count=0,
            edge_count=0,
            lock_counts=(),
        )

    workspace_packages, inventory_errors = _package_inventory(data)
    errors.extend(inventory_errors)
    workspace_names = set(workspace_packages)

    for name, (path, package) in sorted(workspace_packages.items()):
        version = package.get("version")
        if _version_inherits_workspace(version):
            continue
        if version != candidate:
            shown = version if isinstance(version, str) else "<missing>"
            errors.append(f"{path}: package {name} has version {shown}; expected {candidate}")

    python_package = data.python_manifest.get("package")
    python_name = python_package.get("name") if isinstance(python_package, dict) else None
    python_version = python_package.get("version") if isinstance(python_package, dict) else None
    if not isinstance(python_name, str) or not python_name:
        errors.append("crates/rusty-modbus-python/Cargo.toml: missing [package].name")
        python_name = "rusty-modbus-python"
    if python_version != candidate:
        errors.append(
            "crates/rusty-modbus-python/Cargo.toml: "
            f"package version is {python_version!r}; expected {candidate!r}"
        )

    project = data.pyproject.get("project")
    project_version = project.get("version") if isinstance(project, dict) else None
    if project_version != candidate:
        errors.append(
            "crates/rusty-modbus-python/pyproject.toml: "
            f"project version is {project_version!r}; expected {candidate!r}"
        )
    dynamic = project.get("dynamic", []) if isinstance(project, dict) else []
    if isinstance(dynamic, list) and "version" in dynamic:
        errors.append("crates/rusty-modbus-python/pyproject.toml: project version is dynamic")
    build_system = data.pyproject.get("build-system")
    if not isinstance(build_system, dict) or build_system.get("build-backend") != "maturin":
        errors.append(
            "crates/rusty-modbus-python/pyproject.toml: build backend must be maturin"
        )
    tool = data.pyproject.get("tool")
    if not isinstance(tool, dict) or not isinstance(tool.get("maturin"), dict):
        errors.append("crates/rusty-modbus-python/pyproject.toml: missing [tool.maturin]")

    fuzz_package = data.fuzz_manifest.get("package")
    fuzz_name = fuzz_package.get("name") if isinstance(fuzz_package, dict) else None
    fuzz_version = fuzz_package.get("version") if isinstance(fuzz_package, dict) else None
    if not isinstance(fuzz_name, str) or not fuzz_name:
        errors.append("fuzz/Cargo.toml: missing [package].name")
        fuzz_name = "rusty-modbus-fuzz"
    if fuzz_version != "0.0.0":
        errors.append(f"fuzz/Cargo.toml: package version is {fuzz_version!r}; expected '0.0.0'")

    workspace_versions = {name: candidate for name in workspace_names}
    lock_specs = (
        ("Cargo.lock", workspace_versions, workspace_names),
        (
            "crates/rusty-modbus-python/Cargo.lock",
            {**workspace_versions, python_name: candidate},
            {python_name},
        ),
        (
            "fuzz/Cargo.lock",
            {**workspace_versions, fuzz_name: "0.0.0"},
            {fuzz_name},
        ),
    )
    lock_counts = []
    for label, expected_versions, required_names in lock_specs:
        lock_errors, count = _lock_errors(
            label,
            data.locks.get(label, {}),
            expected_versions,
            required_names,
        )
        errors.extend(lock_errors)
        lock_counts.append((label, count))

    if metadata is None:
        return ValidationResult(
            errors=tuple(sorted(set(errors))),
            candidate=candidate,
            package_count=0,
            edge_count=0,
            lock_counts=tuple(lock_counts),
        )

    raw_packages = metadata.get("packages")
    metadata_packages = raw_packages if isinstance(raw_packages, list) else []
    if not isinstance(raw_packages, list):
        errors.append("cargo metadata: missing packages list")
    metadata_names: set[str] = set()
    for package in metadata_packages:
        if isinstance(package, dict) and isinstance(package.get("name"), str):
            metadata_names.add(package["name"])
    for name in sorted(workspace_names - metadata_names):
        errors.append(f"cargo metadata: workspace package {name} is missing")
    for name in sorted(metadata_names - workspace_names):
        errors.append(f"cargo metadata: unexpected workspace package {name}")

    edge_count = 0
    expected_requirement = f"^{candidate}"
    for package in sorted(
        (item for item in metadata_packages if isinstance(item, dict)),
        key=lambda item: str(item.get("name", "")),
    ):
        name = package.get("name")
        version = package.get("version")
        if name in workspace_names and version != candidate:
            errors.append(
                f"cargo metadata: workspace package {name} has version {version}; "
                f"expected {candidate}"
            )
        # An empty list disables publication; null or a nonempty list permits it.
        if package.get("publish") == []:
            continue
        dependencies = package.get("dependencies")
        if not isinstance(dependencies, list):
            errors.append(f"cargo metadata: package {name} has no dependency list")
            continue
        for dependency in dependencies:
            if not isinstance(dependency, dict):
                continue
            if dependency.get("kind") == "dev":
                continue
            if dependency.get("path") is None or dependency.get("name") not in workspace_names:
                continue
            edge_count += 1
            dependency_name = dependency["name"]
            requirement = dependency.get("req")
            edge = f"{name} -> {dependency_name}"
            if requirement == "*":
                errors.append(
                    f"{edge}: missing version requirement; add version = "
                    f'"{candidate}" alongside path'
                )
            elif requirement != expected_requirement:
                errors.append(
                    f"{edge}: version requirement is {requirement!r}; "
                    f"expected {expected_requirement!r}"
                )

    return ValidationResult(
        errors=tuple(sorted(set(errors))),
        candidate=candidate,
        package_count=len(metadata_packages),
        edge_count=edge_count,
        lock_counts=tuple(lock_counts),
    )


def _print_failure(errors: tuple[str, ...]) -> None:
    print("Release version consistency check failed:", file=sys.stderr)
    for error in errors:
        print(f"  - {error}", file=sys.stderr)


def main() -> int:
    try:
        data = load_repository(ROOT)
    except CheckError as error:
        _print_failure((str(error),))
        return 1

    preliminary = validate(data)
    if preliminary.errors:
        _print_failure(preliminary.errors)
        return 1

    try:
        metadata = cargo_metadata(ROOT)
    except CheckError as error:
        _print_failure((str(error),))
        return 1

    result = validate(data, metadata)
    if result.errors:
        _print_failure(result.errors)
        return 1

    locks = ", ".join(f"{path}={count}" for path, count in result.lock_counts)
    print(
        f"OK: release candidate {result.candidate}; "
        f"{result.package_count} workspace packages, "
        f"{result.edge_count} publishable internal normal/build dependencies; "
        f"local lock records: {locks}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
