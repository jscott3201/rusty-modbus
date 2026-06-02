#!/usr/bin/env bash
# One-time local setup: point git at the tracked .githooks/ directory.
# Run once per clone:  bash scripts/install-hooks.sh
#
# .githooks/ is version-controlled (unlike .git/hooks/), so contributors share
# the same gates. Mirrors the CI split:
#   pre-commit -> cargo fmt --check                      (fast)
#   pre-push   -> rust-analyzer --version + clippy       (fast; the full
#                 nextest + doctest gate runs via scripts/test-local.sh
#                 or scripts/ci-pr.sh, not on every push)
#   python     -> scripts/ci-python.sh                    (opt-in wheel,
#                 pytest, stubtest, pyright gate)
#
# Escape hatches: `git commit/push --no-verify` (once) or
# `export RUSTY_MODBUS_SKIP_HOOKS=1` (whole shell session).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push 2>/dev/null || true

echo "core.hooksPath -> .githooks"
echo "  pre-commit: cargo fmt --check"
echo "  pre-push:   rust-analyzer --version + cargo clippy -D warnings"
if command -v cargo-nextest >/dev/null 2>&1; then
  echo "  nextest:    available (scripts/test-local.sh uses it)"
else
  echo "  nextest:    missing; install with: cargo install cargo-nextest --locked"
fi
echo "  full local: scripts/test-local.sh or scripts/ci-pr.sh"
echo "  python:     scripts/ci-python.sh or RUSTY_MODBUS_RUN_PYTHON=full scripts/ci-pr.sh"
echo "Skip once: --no-verify   |   skip session: export RUSTY_MODBUS_SKIP_HOOKS=1"
