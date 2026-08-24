#!/usr/bin/env python3
"""Focused tests for fuzz corpus validation and command construction."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("rusty_modbus_fuzz_tool", ROOT / "scripts/fuzz.py")
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load scripts/fuzz.py")
fuzz = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = fuzz
SPEC.loader.exec_module(fuzz)


class FuzzToolTests(unittest.TestCase):
    def test_canonical_manifest_is_valid(self) -> None:
        entries = fuzz.validate_manifest(ROOT)
        self.assertEqual({entry.target for entry in entries}, set(fuzz.TARGETS))

    def test_replay_uses_sorted_explicit_files_and_fixed_bounds(self) -> None:
        entries = fuzz.validate_manifest(ROOT)
        artifact_dir = ROOT / "fuzz/artifacts/unit"
        command = fuzz.construct_replay_command("mbap_stream", entries, artifact_dir, ROOT)
        separator = command.index("--")
        target_index = command.index("mbap_stream")
        files = command[target_index + 1 : separator]
        self.assertTrue(files)
        self.assertEqual(files, sorted(files))
        self.assertTrue(all(Path(path).is_file() for path in files))
        self.assertNotIn("fuzz/corpus/mbap_stream", files)
        self.assertIn("--features", command)
        self.assertIn("frame", command)
        self.assertIn("-runs=1", command)
        self.assertNotIn("-jobs=1", command)
        self.assertNotIn("-j", command)
        self.assertIn("-print_final_stats=1", command)
        self.assertIn(f"-max_len={fuzz.MAX_INPUT_LENGTH}", command)

    def test_pdu_replay_disables_default_features(self) -> None:
        entries = fuzz.validate_manifest(ROOT)
        command = fuzz.construct_replay_command(
            "pdu_decode", entries, ROOT / "fuzz/artifacts/unit", ROOT
        )
        self.assertIn("--no-default-features", command)
        self.assertNotIn("--features", command)

    def test_campaign_requires_positive_bounds_and_temp_corpus(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory)
            command = fuzz.construct_campaign_command(
                "rtu_frame", corpus, 7, 1234, corpus / "artifacts", ROOT
            )
        self.assertIn(str(corpus), command)
        self.assertIn("-max_total_time=7", command)
        self.assertIn("-seed=1234", command)
        with self.assertRaises(fuzz.FuzzError):
            fuzz.construct_campaign_command(
                "rtu_frame", corpus, 0, 1234, corpus / "artifacts", ROOT
            )

    def test_minimization_failure_is_reported_not_raised(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / "artifacts"
            artifacts.mkdir()
            (artifacts / "crash-input").write_bytes(b"boom")
            log = root / "fuzz.log"
            with mock.patch.object(
                fuzz.subprocess,
                "run",
                side_effect=subprocess.TimeoutExpired(["cargo", "fuzz", "tmin"], 1),
            ):
                errors = fuzz.minimize_artifacts("pdu_decode", artifacts, log, ROOT)
            self.assertEqual(errors, ["crash-input: cargo fuzz tmin timed out"])

    def test_campaign_snapshot_retains_final_temp_corpus(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            corpus = root / "corpus"
            output = root / "output"
            corpus.mkdir()
            output.mkdir()
            (corpus / "seed").write_bytes(b"retained")
            destination = fuzz.copy_campaign_snapshot(corpus, output)
            self.assertEqual((destination / "seed").read_bytes(), b"retained")

    def test_fresh_output_is_marked_and_marker_is_not_an_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory).resolve() / "output"
            prepared = fuzz._prepare_output(output)
            self.assertEqual(prepared, output)
            self.assertEqual(
                (output / fuzz.OUTPUT_MARKER).read_text(encoding="utf-8"),
                fuzz.OUTPUT_MARKER_CONTENT,
            )
            self.assertTrue((output / "artifacts").is_dir())
            self.assertEqual(fuzz._artifact_paths(output), [])

    def test_marked_output_can_be_replaced(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory).resolve() / "output"
            fuzz._prepare_output(output)
            (output / "prior-run").write_bytes(b"replace me")
            fuzz._prepare_output(output)
            self.assertFalse((output / "prior-run").exists())
            self.assertTrue((output / fuzz.OUTPUT_MARKER).is_file())
            self.assertTrue((output / "artifacts").is_dir())

    def test_unmarked_existing_output_is_untouched_and_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            output = root / "output"
            output.mkdir()
            sentinel = output / "keep"
            sentinel.write_bytes(b"valuable")
            with self.assertRaisesRegex(fuzz.FuzzError, "not owned by this tool"):
                fuzz._prepare_output(output)
            self.assertEqual(sentinel.read_bytes(), b"valuable")

            file_output = root / "file-output"
            file_output.write_bytes(b"valuable")
            with self.assertRaisesRegex(fuzz.FuzzError, "must be a directory"):
                fuzz._prepare_output(file_output)
            self.assertEqual(file_output.read_bytes(), b"valuable")

    def test_protected_repository_and_corpus_outputs_are_rejected(self) -> None:
        protected = (
            ROOT.parent,
            ROOT,
            ROOT / "fuzz",
            ROOT / "fuzz/corpus",
            ROOT / "fuzz/corpus/new-output",
            ROOT / "docs/new-output",
        )
        for output in protected:
            with self.subTest(output=output):
                with self.assertRaisesRegex(fuzz.FuzzError, "output path"):
                    fuzz._prepare_output(output)

        with tempfile.TemporaryDirectory() as directory:
            repository = Path(directory).resolve() / "repository"
            protected_output = repository / "fuzz/corpus/owned-looking"
            protected_output.mkdir(parents=True)
            (protected_output / fuzz.OUTPUT_MARKER).write_text(
                fuzz.OUTPUT_MARKER_CONTENT, encoding="utf-8"
            )
            sentinel = protected_output / "keep"
            sentinel.write_bytes(b"valuable")
            with self.assertRaisesRegex(fuzz.FuzzError, "output path"):
                fuzz._prepare_output(protected_output, root=repository)
            self.assertEqual(sentinel.read_bytes(), b"valuable")

    def test_symlink_output_cannot_redirect_replacement(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            owned = root / "owned"
            fuzz._prepare_output(owned)
            sentinel = owned / "keep"
            sentinel.write_bytes(b"valuable")
            link = root / "link"
            try:
                link.symlink_to(owned, target_is_directory=True)
            except OSError as error:
                self.skipTest(f"cannot create directory symlink: {error}")
            with self.assertRaisesRegex(fuzz.FuzzError, "must not contain a symlink"):
                fuzz._prepare_output(link)
            self.assertEqual(sentinel.read_bytes(), b"valuable")

            parent_link = root / "parent-link"
            parent_link.symlink_to(root, target_is_directory=True)
            with self.assertRaisesRegex(fuzz.FuzzError, "must not contain a symlink"):
                fuzz._prepare_output(parent_link / "owned")
            self.assertEqual(sentinel.read_bytes(), b"valuable")

    def test_validator_rejects_extra_and_hash_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_fixture(Path(directory))
            extra = root / "fuzz/corpus/pdu_decode/extra.bin"
            extra.write_bytes(b"extra")
            with self.assertRaisesRegex(fuzz.FuzzError, "untracked corpus files"):
                fuzz.validate_manifest(root)
            extra.unlink()
            corpus = root / "fuzz/corpus/pdu_decode/APP-001__pdu_decode__case.bin"
            corpus.write_bytes(b"changed")
            with self.assertRaisesRegex(fuzz.FuzzError, "sha256 mismatch"):
                fuzz.validate_manifest(root)

    def test_validator_rejects_noncanonical_entry_order(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_fixture(Path(directory))
            path = root / "fuzz/corpus/manifest.json"
            manifest = json.loads(path.read_text(encoding="utf-8"))
            manifest["entries"].reverse()
            path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
            with self.assertRaisesRegex(fuzz.FuzzError, "canonical target/path order"):
                fuzz.validate_manifest(root)

    def test_validator_rejects_unknown_requirement_and_bad_target_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_fixture(Path(directory))
            path = root / "fuzz/corpus/manifest.json"
            manifest = json.loads(path.read_text(encoding="utf-8"))
            manifest["targets"] = list(reversed(fuzz.TARGETS))
            path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
            with self.assertRaisesRegex(fuzz.FuzzError, "target inventory"):
                fuzz.validate_manifest(root)

            root = self.make_fixture(Path(directory), replace=True)
            path = root / "fuzz/corpus/manifest.json"
            manifest = json.loads(path.read_text(encoding="utf-8"))
            manifest["entries"][0]["requirement_ids"] = ["APP-999"]
            path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
            with self.assertRaisesRegex(fuzz.FuzzError, "unknown requirement"):
                fuzz.validate_manifest(root)

    def make_fixture(self, root: Path, *, replace: bool = False) -> Path:
        if replace:
            import shutil

            shutil.rmtree(root)
            root.mkdir()
        (root / "docs/conformance").mkdir(parents=True)
        (root / "docs/conformance/ledger.json").write_text(
            json.dumps({"requirements": [{"id": "APP-001"}]}) + "\n", encoding="utf-8"
        )
        entries = []
        for target in fuzz.TARGETS:
            target_dir = root / "fuzz/corpus" / target
            target_dir.mkdir(parents=True)
            relative = f"{target}/APP-001__{target}__case.bin"
            contents = target.encode("ascii")
            (root / "fuzz/corpus" / relative).write_bytes(contents)
            entries.append(
                {
                    "target": target,
                    "path": relative,
                    "requirement_ids": ["APP-001"],
                    "class": "valid",
                    "source": "unit fixture",
                    "contract": "unit fixture contract",
                    "sha256": hashlib.sha256(contents).hexdigest(),
                }
            )
        manifest = {"schema_version": 1, "targets": list(fuzz.TARGETS), "entries": entries}
        (root / "fuzz/corpus/manifest.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        return root


if __name__ == "__main__":
    unittest.main(verbosity=2)
