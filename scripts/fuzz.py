#!/usr/bin/env python3
"""Validate, replay, and run bounded campaigns for the retained fuzz corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any, Sequence

ROOT = Path(__file__).resolve().parents[1]
FUZZ_DIR = ROOT / "fuzz"
CORPUS_DIR = FUZZ_DIR / "corpus"
MANIFEST_PATH = CORPUS_DIR / "manifest.json"
TOOLCHAIN_PATH = FUZZ_DIR / "rust-toolchain.toml"

NIGHTLY = "nightly-2026-08-15"
CARGO_FUZZ_VERSION = "0.13.2"
EXPECTED_NIGHTLY_RELEASE = "1.99.0-nightly"
EXPECTED_NIGHTLY_COMMIT_DATE = "2026-08-14"
TARGETS = ("pdu_decode", "mbap_stream", "rtu_assembler", "rtu_frame", "rtu_tcp_stream")
TARGET_FEATURES = {
    "pdu_decode": None,
    "mbap_stream": "frame",
    "rtu_assembler": "assembler",
    "rtu_frame": "frame",
    "rtu_tcp_stream": "frame",
}
CLASSES = frozenset({"valid", "malformed", "boundary", "regression"})
ENTRY_KEYS = (
    "target",
    "path",
    "requirement_ids",
    "class",
    "source",
    "contract",
    "sha256",
)
TARGET_MAX_FILE_SIZE = {
    "pdu_decode": 254,
    "mbap_stream": 2048,
    "rtu_assembler": 2048,
    "rtu_frame": 257,
    "rtu_tcp_stream": 2048,
}
REPLAY_SEEDS = {
    "pdu_decode": 3_230_003_001,
    "mbap_stream": 3_230_003_002,
    "rtu_assembler": 3_230_003_003,
    "rtu_frame": 3_230_003_004,
    "rtu_tcp_stream": 3_230_003_005,
}
MAX_INPUT_LENGTH = 2048
INPUT_TIMEOUT_SECONDS = 2
RSS_LIMIT_MB = 2048
MAX_CAMPAIGN_SECONDS = 3600
TMIN_RUNS = 64
TMIN_WALL_SECONDS = 45
OUTPUT_MARKER = ".rusty-modbus-fuzz-output"
OUTPUT_MARKER_CONTENT = "rusty-modbus-fuzz-output-v1\n"


class FuzzError(Exception):
    """Raised when fuzz configuration or retained evidence is invalid."""


@dataclass(frozen=True)
class CorpusEntry:
    target: str
    path: str
    requirement_ids: tuple[str, ...]
    case_class: str
    source: str
    contract: str
    sha256: str


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(64 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise FuzzError(f"cannot load {path}: {error}") from error


def _requirement_inventory(root: Path) -> tuple[str, ...]:
    ledger_path = root / "docs/conformance/ledger.json"
    ledger = _load_json(ledger_path)
    if not isinstance(ledger, dict) or not isinstance(ledger.get("requirements"), list):
        raise FuzzError(f"{ledger_path} has no requirement inventory")
    identifiers: list[str] = []
    for item in ledger["requirements"]:
        identifier = item.get("id") if isinstance(item, dict) else None
        if not isinstance(identifier, str) or not identifier:
            raise FuzzError(f"{ledger_path} contains an invalid requirement ID")
        identifiers.append(identifier)
    inventory = tuple(identifiers)
    if not inventory:
        raise FuzzError(f"{ledger_path} has an empty requirement inventory")
    if len(inventory) != len(set(inventory)):
        raise FuzzError(f"{ledger_path} repeats a requirement ID")
    return inventory


def validate_manifest(root: Path = ROOT) -> list[CorpusEntry]:
    corpus_dir = root / "fuzz/corpus"
    manifest_path = corpus_dir / "manifest.json"
    manifest = _load_json(manifest_path)
    if not isinstance(manifest, dict):
        raise FuzzError("fuzz corpus manifest root must be an object")
    if tuple(manifest) != ("schema_version", "targets", "entries"):
        raise FuzzError("fuzz corpus manifest keys or key order are not canonical")
    if manifest["schema_version"] != 1:
        raise FuzzError("fuzz corpus manifest schema_version must be 1")
    if manifest["targets"] != list(TARGETS):
        raise FuzzError("fuzz corpus target inventory or order is not canonical")
    raw_entries = manifest["entries"]
    if not isinstance(raw_entries, list) or not raw_entries:
        raise FuzzError("fuzz corpus manifest entries must be a nonempty list")

    requirement_inventory = _requirement_inventory(root)
    valid_requirements = set(requirement_inventory)
    target_rank = {target: index for index, target in enumerate(TARGETS)}
    entries: list[CorpusEntry] = []
    seen_paths: set[str] = set()

    for index, item in enumerate(raw_entries):
        label = f"fuzz corpus manifest entries[{index}]"
        if not isinstance(item, dict):
            raise FuzzError(f"{label} must be an object")
        if tuple(item) != ENTRY_KEYS:
            raise FuzzError(f"{label} keys or key order are not canonical")

        target = item["target"]
        path_value = item["path"]
        requirement_ids = item["requirement_ids"]
        case_class = item["class"]
        source = item["source"]
        contract = item["contract"]
        digest = item["sha256"]
        if target not in TARGETS:
            raise FuzzError(f"{label}.target is invalid: {target!r}")
        if not isinstance(path_value, str) or not path_value:
            raise FuzzError(f"{label}.path must be nonblank")
        relative = PurePosixPath(path_value)
        if relative.is_absolute() or ".." in relative.parts or relative.as_posix() != path_value:
            raise FuzzError(f"{label}.path is not a normalized relative path")
        if len(relative.parts) != 2 or relative.parts[0] != target:
            raise FuzzError(f"{label}.path must be directly under its target directory")
        if path_value in seen_paths:
            raise FuzzError(f"fuzz corpus manifest repeats path {path_value}")
        seen_paths.add(path_value)

        if not isinstance(requirement_ids, list) or not requirement_ids:
            raise FuzzError(f"{label}.requirement_ids must be a nonempty list")
        if requirement_ids != sorted(requirement_ids) or len(requirement_ids) != len(
            set(requirement_ids)
        ):
            raise FuzzError(f"{label}.requirement_ids must be sorted and unique")
        unknown = [item for item in requirement_ids if item not in valid_requirements]
        if unknown:
            raise FuzzError(f"{label} has unknown requirement IDs: {', '.join(unknown)}")
        filename_prefix = "_".join(requirement_ids) + f"__{target}__"
        case_name = relative.name.removeprefix(filename_prefix).removesuffix(".bin")
        if (
            not relative.name.startswith(filename_prefix)
            or not relative.name.endswith(".bin")
            or not case_name
            or any(character not in "abcdefghijklmnopqrstuvwxyz0123456789-" for character in case_name)
        ):
            raise FuzzError(f"{label}.path does not use the requirement-target-case filename form")
        if case_class not in CLASSES:
            raise FuzzError(f"{label}.class is invalid: {case_class!r}")
        for field, value in (("source", source), ("contract", contract)):
            if not isinstance(value, str) or not value.strip():
                raise FuzzError(f"{label}.{field} must be nonblank")
        if (
            not isinstance(digest, str)
            or len(digest) != 64
            or any(character not in "0123456789abcdef" for character in digest)
        ):
            raise FuzzError(f"{label}.sha256 must be lowercase hexadecimal")

        path = corpus_dir / relative
        if path.is_symlink() or not path.is_file():
            raise FuzzError(f"{label}.path is not a regular file: {path_value}")
        size = path.stat().st_size
        if size < 1 or size > TARGET_MAX_FILE_SIZE[target]:
            raise FuzzError(
                f"{label}.path size {size} is outside 1..={TARGET_MAX_FILE_SIZE[target]}"
            )
        actual_digest = _sha256(path)
        if actual_digest != digest:
            raise FuzzError(
                f"{label}.sha256 mismatch: expected {digest}, found {actual_digest}"
            )
        entries.append(
            CorpusEntry(
                target=target,
                path=path_value,
                requirement_ids=tuple(requirement_ids),
                case_class=case_class,
                source=source,
                contract=contract,
                sha256=digest,
            )
        )

    expected_order = sorted(entries, key=lambda item: (target_rank[item.target], item.path))
    if entries != expected_order:
        raise FuzzError("fuzz corpus manifest entries are not in canonical target/path order")
    counts = {target: 0 for target in TARGETS}
    for entry in entries:
        counts[entry.target] += 1
    missing_targets = [target for target, count in counts.items() if count == 0]
    if missing_targets:
        raise FuzzError(f"fuzz corpus has no entries for: {', '.join(missing_targets)}")

    actual_files = {
        path.relative_to(corpus_dir).as_posix()
        for path in corpus_dir.rglob("*")
        if path.is_file() and path != manifest_path
    }
    extras = sorted(actual_files - seen_paths)
    missing = sorted(seen_paths - actual_files)
    if extras:
        raise FuzzError(f"untracked corpus files are absent from manifest: {', '.join(extras)}")
    if missing:
        raise FuzzError(f"manifest corpus files are missing: {', '.join(missing)}")
    return entries


def entries_for_target(entries: Sequence[CorpusEntry], target: str) -> list[CorpusEntry]:
    selected = sorted((entry for entry in entries if entry.target == target), key=lambda x: x.path)
    if not selected:
        raise FuzzError(f"target {target} has no retained corpus inputs")
    return selected


def _feature_arguments(target: str) -> list[str]:
    if target not in TARGET_FEATURES:
        raise FuzzError(f"unknown fuzz target {target}")
    feature = TARGET_FEATURES[target]
    if feature is None:
        return ["--no-default-features"]
    return ["--features", feature]


def _common_run_prefix(target: str, root: Path) -> list[str]:
    return [
        "cargo",
        f"+{NIGHTLY}",
        "fuzz",
        "run",
        "--fuzz-dir",
        str((root / "fuzz").relative_to(root)),
        "--target-dir",
        str((root / "fuzz/target").relative_to(root)),
        *_feature_arguments(target),
        target,
    ]


def _libfuzzer_bounds(seed: int, artifact_dir: Path) -> list[str]:
    return [
        f"-timeout={INPUT_TIMEOUT_SECONDS}",
        f"-seed={seed}",
        f"-rss_limit_mb={RSS_LIMIT_MB}",
        f"-max_len={MAX_INPUT_LENGTH}",
        "-print_final_stats=1",
        f"-artifact_prefix={artifact_dir}{os.sep}",
    ]


def construct_replay_command(
    target: str,
    entries: Sequence[CorpusEntry],
    artifact_dir: Path,
    root: Path = ROOT,
) -> list[str]:
    selected = entries_for_target(entries, target)
    files = [str((root / "fuzz/corpus" / entry.path).relative_to(root)) for entry in selected]
    return [
        *_common_run_prefix(target, root),
        *files,
        "--",
        "-runs=1",
        *_libfuzzer_bounds(REPLAY_SEEDS[target], artifact_dir),
    ]


def construct_campaign_command(
    target: str,
    corpus: Path,
    seconds: int,
    seed: int,
    artifact_dir: Path,
    root: Path = ROOT,
) -> list[str]:
    if seconds < 1 or seconds > MAX_CAMPAIGN_SECONDS:
        raise FuzzError(f"campaign seconds must be in 1..={MAX_CAMPAIGN_SECONDS}")
    if seed < 1 or seed > 0xFFFF_FFFF:
        raise FuzzError("campaign seed must be in 1..=4294967295")
    return [
        *_common_run_prefix(target, root),
        str(corpus),
        "--",
        f"-max_total_time={seconds}",
        *_libfuzzer_bounds(seed, artifact_dir),
    ]


def construct_tmin_command(
    target: str, artifact: Path, artifact_dir: Path, root: Path = ROOT
) -> list[str]:
    return [
        "cargo",
        f"+{NIGHTLY}",
        "fuzz",
        "tmin",
        "--fuzz-dir",
        str((root / "fuzz").relative_to(root)),
        "--target-dir",
        str((root / "fuzz/target").relative_to(root)),
        "-r",
        str(TMIN_RUNS),
        *_feature_arguments(target),
        target,
        str(artifact),
        "--",
        f"-max_total_time={TMIN_WALL_SECONDS - 5}",
        f"-timeout={INPUT_TIMEOUT_SECONDS}",
        f"-rss_limit_mb={RSS_LIMIT_MB}",
        f"-max_len={MAX_INPUT_LENGTH}",
        "-print_final_stats=1",
        f"-artifact_prefix={artifact_dir}{os.sep}",
    ]


def _run_checked(command: Sequence[str], root: Path = ROOT) -> str:
    result = subprocess.run(
        list(command),
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
    )
    output = result.stdout + result.stderr
    if result.returncode != 0:
        raise FuzzError(f"command failed ({result.returncode}): {shlex.join(command)}\n{output}")
    return output


def check_environment(root: Path = ROOT, *, metadata: bool = True) -> None:
    try:
        toolchain = tomllib.loads((root / "fuzz/rust-toolchain.toml").read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise FuzzError(f"cannot read fuzz toolchain pin: {error}") from error
    configured = toolchain.get("toolchain")
    if not isinstance(configured, dict):
        raise FuzzError("fuzz rust-toolchain.toml has no [toolchain] table")
    if configured.get("channel") != NIGHTLY:
        raise FuzzError(f"fuzz toolchain must pin {NIGHTLY}")
    components = configured.get("components")
    if components != ["rust-src", "rustfmt"]:
        raise FuzzError("fuzz toolchain components must be rust-src and rustfmt in order")

    rustc = _run_checked(["rustc", f"+{NIGHTLY}", "--version", "--verbose"], root)
    if (
        f"release: {EXPECTED_NIGHTLY_RELEASE}" not in rustc
        or f"commit-date: {EXPECTED_NIGHTLY_COMMIT_DATE}" not in rustc
    ):
        raise FuzzError(f"{NIGHTLY} does not match its recorded rustc release\n{rustc}")
    fuzz_version = _run_checked(["cargo", f"+{NIGHTLY}", "fuzz", "--version"], root).strip()
    if fuzz_version != f"cargo-fuzz {CARGO_FUZZ_VERSION}":
        raise FuzzError(
            f"cargo-fuzz must be {CARGO_FUZZ_VERSION}, found {fuzz_version or '<empty>'}"
        )
    installed = _run_checked(
        ["rustup", "component", "list", "--toolchain", NIGHTLY, "--installed"], root
    ).splitlines()
    for component in ("rust-src", "rustfmt"):
        if not any(line == component or line.startswith(f"{component}-") for line in installed):
            raise FuzzError(f"{component} is not installed for {NIGHTLY}")
    if metadata:
        _run_checked(
            [
                "cargo",
                f"+{NIGHTLY}",
                "metadata",
                "--manifest-path",
                "fuzz/Cargo.toml",
                "--locked",
                "--format-version",
                "1",
                "--no-deps",
            ],
            root,
        )


def _immutable_snapshot(entries: Sequence[CorpusEntry], root: Path) -> dict[str, str]:
    relative_paths = ["Cargo.lock", "fuzz/Cargo.lock", "fuzz/corpus/manifest.json"]
    relative_paths.extend(f"fuzz/corpus/{entry.path}" for entry in entries)
    snapshot: dict[str, str] = {}
    for relative in relative_paths:
        path = root / relative
        if not path.is_file():
            raise FuzzError(f"immutable fuzz input is missing: {relative}")
        snapshot[relative] = _sha256(path)
    return snapshot


def _changed_snapshot(before: dict[str, str], root: Path) -> list[str]:
    changed = []
    for relative, expected in before.items():
        path = root / relative
        if not path.is_file() or _sha256(path) != expected:
            changed.append(relative)
    return changed


def _run_logged(command: Sequence[str], log_path: Path, root: Path, timeout: int) -> int:
    with log_path.open("w", encoding="utf-8", newline="\n") as log:
        log.write(f"$ {shlex.join(command)}\n")
        log.flush()
        try:
            result = subprocess.run(
                list(command),
                cwd=root,
                check=False,
                stdout=log,
                stderr=subprocess.STDOUT,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            log.write(f"\nwrapper timeout after {timeout} seconds\n")
            return 124
    return result.returncode


def minimize_artifacts(
    target: str, artifact_dir: Path, log_path: Path, root: Path = ROOT
) -> list[str]:
    failures: list[str] = []
    candidates = sorted(path for path in artifact_dir.iterdir() if path.is_file())
    if not candidates:
        return failures
    minimized_dir = artifact_dir / "minimized"
    minimized_dir.mkdir(parents=True, exist_ok=True)
    for artifact in candidates:
        command = construct_tmin_command(target, artifact, minimized_dir, root)
        with log_path.open("a", encoding="utf-8", newline="\n") as log:
            log.write(f"\n$ {shlex.join(command)}\n")
            log.flush()
            try:
                result = subprocess.run(
                    command,
                    cwd=root,
                    check=False,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    text=True,
                    timeout=TMIN_WALL_SECONDS,
                )
                if result.returncode != 0:
                    failures.append(f"{artifact.name}: cargo fuzz tmin exited {result.returncode}")
            except subprocess.TimeoutExpired:
                failures.append(f"{artifact.name}: cargo fuzz tmin timed out")
            except OSError as error:
                failures.append(f"{artifact.name}: cargo fuzz tmin failed to start: {error}")
    return failures


def _git_commit(root: Path) -> str:
    return _run_checked(["git", "rev-parse", "HEAD"], root).strip()


def _artifact_paths(output_dir: Path) -> list[str]:
    excluded = {output_dir / OUTPUT_MARKER, output_dir / "metadata.json"}
    return sorted(
        path.relative_to(output_dir).as_posix()
        for path in output_dir.rglob("*")
        if path.is_file() and path not in excluded
    )


def _write_metadata(
    output_dir: Path,
    *,
    mode: str,
    target: str,
    commit: str,
    command: Sequence[str],
    seed: int,
    requirement_ids: Sequence[str],
    returncode: int,
    minimization_errors: Sequence[str],
) -> None:
    metadata = {
        "schema_version": 1,
        "mode": mode,
        "status": "pass" if returncode == 0 else "failure",
        "target": target,
        "commit": commit,
        "toolchain": NIGHTLY,
        "cargo_fuzz_version": CARGO_FUZZ_VERSION,
        "command": shlex.join(command),
        "seed": seed,
        "requirement_ids": list(requirement_ids),
        "returncode": returncode,
        "artifact_paths": _artifact_paths(output_dir),
        "minimization_errors": list(minimization_errors),
    }
    (output_dir / "metadata.json").write_text(
        json.dumps(metadata, indent=2) + "\n", encoding="utf-8", newline="\n"
    )


def _symlink_component(path: Path) -> Path | None:
    absolute = Path(os.path.abspath(path))
    current = Path(absolute.anchor)
    for part in absolute.parts[1:]:
        current /= part
        if current.is_symlink():
            return current
        if not current.exists():
            break
    return None


def _prepare_output(path: Path, root: Path = ROOT) -> Path:
    requested = path.expanduser()
    symlink = _symlink_component(requested)
    if symlink is not None:
        raise FuzzError(f"output path must not contain a symlink: {symlink}")
    resolved = requested.resolve(strict=False)
    repository = root.resolve()
    generated_root = (repository / "fuzz/artifacts").resolve()

    if resolved == repository or repository.is_relative_to(resolved):
        raise FuzzError(f"output path must not be the repository or its ancestor: {resolved}")
    if resolved.is_relative_to(repository) and not resolved.is_relative_to(generated_root):
        raise FuzzError(
            f"output path inside the repository must be under fuzz/artifacts: {resolved}"
        )

    marker = resolved / OUTPUT_MARKER
    if resolved.exists():
        if not resolved.is_dir():
            raise FuzzError(f"output path must be a directory: {resolved}")
        if marker.is_symlink() or not marker.is_file():
            raise FuzzError(f"existing output directory is not owned by this tool: {resolved}")
        try:
            marker_contents = marker.read_text(encoding="utf-8")
        except OSError as error:
            raise FuzzError(f"cannot read output ownership marker {marker}: {error}") from error
        if marker_contents != OUTPUT_MARKER_CONTENT:
            raise FuzzError(f"existing output directory has an invalid ownership marker: {resolved}")
        try:
            shutil.rmtree(resolved)
        except OSError as error:
            raise FuzzError(f"cannot replace owned output directory {resolved}: {error}") from error

    try:
        resolved.mkdir(parents=True)
        (resolved / "artifacts").mkdir()
        marker.write_text(OUTPUT_MARKER_CONTENT, encoding="utf-8", newline="\n")
    except OSError as error:
        raise FuzzError(f"cannot prepare output directory {resolved}: {error}") from error
    return resolved


def copy_campaign_snapshot(corpus: Path, output_dir: Path) -> Path:
    destination = output_dir / "generated-corpus"
    shutil.copytree(corpus, destination)
    return destination


def _run_one(
    mode: str,
    target: str,
    entries: Sequence[CorpusEntry],
    *,
    output_dir: Path,
    seconds: int | None = None,
    seed: int | None = None,
    root: Path = ROOT,
) -> int:
    output_dir = _prepare_output(output_dir, root)
    artifact_dir = output_dir / "artifacts"
    log_path = output_dir / "fuzz.log"
    selected = entries_for_target(entries, target)
    requirement_ids = sorted(
        {requirement for entry in selected for requirement in entry.requirement_ids}
    )
    before = _immutable_snapshot(entries, root)

    if mode == "replay":
        selected_seed = REPLAY_SEEDS[target]
        command = construct_replay_command(target, entries, artifact_dir, root)
        wall_timeout = max(300, len(selected) * (INPUT_TIMEOUT_SECONDS + 2))
        returncode = _run_logged(command, log_path, root, wall_timeout)
    elif mode == "campaign":
        if seconds is None or seed is None:
            raise FuzzError("campaign requires seconds and seed")
        selected_seed = seed
        with tempfile.TemporaryDirectory(prefix=f"rusty-modbus-{target}-corpus-") as directory:
            temporary_corpus = Path(directory)
            for entry in selected:
                shutil.copy2(root / "fuzz/corpus" / entry.path, temporary_corpus / Path(entry.path).name)
            command = construct_campaign_command(
                target, temporary_corpus, seconds, selected_seed, artifact_dir, root
            )
            returncode = _run_logged(command, log_path, root, seconds + 300)
            copy_campaign_snapshot(temporary_corpus, output_dir)
    else:
        raise FuzzError(f"unknown fuzz mode {mode}")

    changed = _changed_snapshot(before, root)
    if changed:
        with log_path.open("a", encoding="utf-8", newline="\n") as log:
            log.write("\nimmutable inputs changed: " + ", ".join(changed) + "\n")
        returncode = returncode or 1
    minimization_errors: list[str] = []
    if returncode != 0:
        minimization_errors = minimize_artifacts(target, artifact_dir, log_path, root)
    _write_metadata(
        output_dir,
        mode=mode,
        target=target,
        commit=_git_commit(root),
        command=command,
        seed=selected_seed,
        requirement_ids=requirement_ids,
        returncode=returncode,
        minimization_errors=minimization_errors,
    )
    print(log_path.read_text(encoding="utf-8"), end="")
    print(f"fuzz {mode} metadata: {output_dir / 'metadata.json'}")
    return returncode


def _parse_seed(value: str) -> int:
    try:
        seed = int(value, 0)
    except ValueError as error:
        raise argparse.ArgumentTypeError("seed must be an integer") from error
    if seed < 1 or seed > 0xFFFF_FFFF:
        raise argparse.ArgumentTypeError("seed must be in 1..=4294967295")
    return seed


def _parse_seconds(value: str) -> int:
    try:
        seconds = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("seconds must be an integer") from error
    if seconds < 1 or seconds > MAX_CAMPAIGN_SECONDS:
        raise argparse.ArgumentTypeError(f"seconds must be in 1..={MAX_CAMPAIGN_SECONDS}")
    return seconds


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("check", help="validate pins, lock consistency, and retained corpus")

    replay = subparsers.add_parser("replay", help="replay explicit retained inputs once")
    replay.add_argument("targets", nargs="*", choices=TARGETS)
    replay.add_argument("--output", type=Path)

    campaign = subparsers.add_parser("campaign", help="run one time-bounded target campaign")
    campaign.add_argument("target", choices=TARGETS)
    campaign.add_argument("--seconds", type=_parse_seconds, required=True)
    campaign.add_argument("--seed", type=_parse_seed, required=True)
    campaign.add_argument("--output", type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        entries = validate_manifest(ROOT)
        check_environment(ROOT)
        if args.command == "check":
            counts = {target: len(entries_for_target(entries, target)) for target in TARGETS}
            summary = ", ".join(f"{target}={counts[target]}" for target in TARGETS)
            print(
                f"fuzz configuration valid: {NIGHTLY}, cargo-fuzz {CARGO_FUZZ_VERSION}; {summary}"
            )
            return 0
        if args.command == "replay":
            targets = args.targets or list(TARGETS)
            status = 0
            for target in targets:
                output = (
                    args.output / target
                    if args.output
                    else FUZZ_DIR / "artifacts" / f"replay-{target}"
                )
                result = _run_one("replay", target, entries, output_dir=output)
                status = status or result
            return status
        if args.command == "campaign":
            output = args.output or FUZZ_DIR / "artifacts" / f"campaign-{args.target}"
            return _run_one(
                "campaign",
                args.target,
                entries,
                output_dir=output,
                seconds=args.seconds,
                seed=args.seed,
            )
        raise FuzzError(f"unsupported command {args.command}")
    except FuzzError as error:
        print(f"fuzz: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
