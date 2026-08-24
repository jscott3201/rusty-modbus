#!/usr/bin/env python3
"""Focused tests for the release version consistency guard."""

from __future__ import annotations

import copy
import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "rusty_modbus_publish_versions", ROOT / "scripts/check-publish-versions.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load scripts/check-publish-versions.py")
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)

CANDIDATE = "0.1.1"


def local_package(name: str, version: str = CANDIDATE) -> dict[str, str]:
    return {"name": name, "version": version}


class PublishVersionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.data = checker.RepositoryData(
            root_manifest={
                "workspace": {
                    "members": ["crates/*"],
                    "package": {"version": CANDIDATE},
                }
            },
            workspace_manifests=(
                (
                    "crates/rusty-modbus-types/Cargo.toml",
                    {
                        "package": {
                            "name": "rusty-modbus-types",
                            "version": {"workspace": True},
                        }
                    },
                ),
                (
                    "crates/rusty-modbus-client/Cargo.toml",
                    {
                        "package": {
                            "name": "rusty-modbus-client",
                            "version": {"workspace": True},
                        }
                    },
                ),
            ),
            python_manifest={
                "package": {"name": "rusty-modbus-python", "version": CANDIDATE}
            },
            pyproject={
                "build-system": {"build-backend": "maturin"},
                "project": {"name": "rusty_modbus", "version": CANDIDATE},
                "tool": {"maturin": {"features": ["pyo3/extension-module"]}},
            },
            fuzz_manifest={
                "package": {"name": "rusty-modbus-fuzz", "version": "0.0.0"}
            },
            locks={
                "Cargo.lock": {
                    "package": [
                        local_package("rusty-modbus-client"),
                        local_package("rusty-modbus-types"),
                        {
                            "name": "rusty-modbus-types",
                            "version": "0.1.0",
                            "source": "registry+https://github.com/rust-lang/crates.io-index",
                        },
                    ]
                },
                "crates/rusty-modbus-python/Cargo.lock": {
                    "package": [
                        local_package("rusty-modbus-python"),
                        local_package("rusty-modbus-types"),
                    ]
                },
                "fuzz/Cargo.lock": {
                    "package": [
                        local_package("rusty-modbus-fuzz", "0.0.0"),
                        local_package("rusty-modbus-types"),
                    ]
                },
            },
        )
        self.metadata = {
            "packages": [
                {
                    "name": "rusty-modbus-types",
                    "version": CANDIDATE,
                    "publish": None,
                    "dependencies": [],
                },
                {
                    "name": "rusty-modbus-client",
                    "version": CANDIDATE,
                    "publish": ["crates-io"],
                    "dependencies": [
                        {
                            "name": "rusty-modbus-types",
                            "kind": None,
                            "path": "/fixture/rusty-modbus-types",
                            "req": f"^{CANDIDATE}",
                        },
                        {
                            "name": "rusty-modbus-types",
                            "kind": "build",
                            "path": "/fixture/rusty-modbus-types",
                            "req": f"^{CANDIDATE}",
                        },
                        {
                            "name": "external-at-same-version",
                            "kind": None,
                            "path": None,
                            "req": f"^{CANDIDATE}",
                        },
                    ],
                },
            ]
        }

    def validate(self):
        return checker.validate(self.data, self.metadata)

    def test_consistent_candidate_succeeds(self) -> None:
        result = self.validate()
        self.assertEqual(result.errors, ())
        self.assertEqual(result.package_count, 2)
        self.assertEqual(result.edge_count, 2)
        self.assertEqual(dict(result.lock_counts)["Cargo.lock"], 2)

    def test_mixed_workspace_package_version_fails(self) -> None:
        self.metadata["packages"][1]["version"] = "0.1.0"
        self.assertIn(
            "cargo metadata: workspace package rusty-modbus-client has version 0.1.0; "
            "expected 0.1.1",
            self.validate().errors,
        )

    def test_mixed_python_versions_fail(self) -> None:
        for field in ("cargo", "pyproject"):
            with self.subTest(field=field):
                data = copy.deepcopy(self.data)
                if field == "cargo":
                    data.python_manifest["package"]["version"] = "0.1.0"
                    expected = (
                        "crates/rusty-modbus-python/Cargo.toml: package version is "
                        "'0.1.0'; expected '0.1.1'"
                    )
                else:
                    data.pyproject["project"]["version"] = "0.1.0"
                    expected = (
                        "crates/rusty-modbus-python/pyproject.toml: project version is "
                        "'0.1.0'; expected '0.1.1'"
                    )
                self.assertIn(expected, checker.validate(data, self.metadata).errors)

    def test_missing_internal_requirement_fails(self) -> None:
        self.metadata["packages"][1]["dependencies"][0]["req"] = "*"
        self.assertIn(
            "rusty-modbus-client -> rusty-modbus-types: missing version requirement; "
            'add version = "0.1.1" alongside path',
            self.validate().errors,
        )

    def test_stale_build_requirement_fails(self) -> None:
        self.metadata["packages"][1]["dependencies"][1]["req"] = "^0.1.0"
        self.assertIn(
            "rusty-modbus-client -> rusty-modbus-types: version requirement is "
            "'^0.1.0'; expected '^0.1.1'",
            self.validate().errors,
        )

    def test_stale_local_lock_record_fails(self) -> None:
        self.data.locks["fuzz/Cargo.lock"]["package"][1]["version"] = "0.1.0"
        self.assertIn(
            "fuzz/Cargo.lock: local package rusty-modbus-types has version 0.1.0; "
            "expected 0.1.1",
            self.validate().errors,
        )

    def test_registry_record_with_local_name_is_ignored(self) -> None:
        result = self.validate()
        self.assertFalse(
            any("registry" in error or "rusty-modbus-types has version 0.1.0" in error for error in result.errors)
        )

    def test_fuzz_package_must_remain_zero(self) -> None:
        self.data.fuzz_manifest["package"]["version"] = CANDIDATE
        self.assertIn(
            "fuzz/Cargo.toml: package version is '0.1.1'; expected '0.0.0'",
            self.validate().errors,
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
