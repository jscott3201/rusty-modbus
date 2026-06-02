#!/usr/bin/env bash
# Local mirror of .github/workflows/ci.yml for PRs into dev.
#
# Usage:
#   scripts/ci-pr.sh
#   RUSTY_MODBUS_RUN_DENY=always scripts/ci-pr.sh
#   RUSTY_MODBUS_RUN_DENY=never scripts/ci-pr.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

run() {
  echo
  echo "==> $*"
  "$@"
}

have() {
  command -v "$1" >/dev/null 2>&1
}

deps_changed() {
  local base
  if git rev-parse --verify origin/dev >/dev/null 2>&1; then
    base="$(git merge-base HEAD origin/dev)"
    git diff --name-only "$base" -- \
      Cargo.lock Cargo.toml 'crates/*/Cargo.toml' benchmarks/Cargo.toml deny.toml |
      grep -q .
  else
    git diff --name-only HEAD -- \
      Cargo.lock Cargo.toml 'crates/*/Cargo.toml' benchmarks/Cargo.toml deny.toml |
      grep -q .
  fi
}

run cargo fmt --all --check
run rust-analyzer --version
run cargo clippy --workspace --all-targets --locked -- -D warnings

export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"

if have cargo-nextest; then
  run cargo nextest run --workspace --locked --profile ci
else
  echo
  echo "==> cargo-nextest missing; falling back to cargo test --workspace --locked"
  run cargo test --workspace --locked
fi

run cargo test --workspace --locked --doc

(
  cd crates/rusty-modbus-python
  run cargo clippy --all-targets --locked -- -D warnings
)

case "${RUSTY_MODBUS_RUN_DENY:-auto}" in
  always)
    run cargo deny check bans licenses sources
    ;;
  never)
    echo
    echo "==> cargo-deny skipped (RUSTY_MODBUS_RUN_DENY=never)"
    ;;
  auto)
    if deps_changed; then
      run cargo deny check bans licenses sources
    else
      echo
      echo "==> cargo-deny skipped (no dependency manifest changes)"
    fi
    ;;
  *)
    echo "RUSTY_MODBUS_RUN_DENY must be auto, always, or never" >&2
    exit 2
    ;;
esac
