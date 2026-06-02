#!/usr/bin/env bash
# Local mirror of .github/workflows/ci.yml for PRs into dev.
#
# Usage:
#   scripts/ci-pr.sh
#   RUSTY_MODBUS_RUN_PYTHON=always scripts/ci-pr.sh
#   RUSTY_MODBUS_RUN_PYTHON=never scripts/ci-pr.sh
#   RUSTY_MODBUS_RUN_DENY=always scripts/ci-pr.sh
#   RUSTY_MODBUS_RUN_DENY=never scripts/ci-pr.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

run() {
  echo
  echo "==> $*"
  "$@"
}

changed_since_base() {
  local base
  if git rev-parse --verify origin/dev >/dev/null 2>&1; then
    base="$(git merge-base HEAD origin/dev)"
    git diff --name-only "$base" -- "$@" | grep -q .
  else
    git diff --name-only HEAD -- "$@" | grep -q .
  fi
}

deps_changed() {
  changed_since_base \
    Cargo.lock Cargo.toml 'crates/*/Cargo.toml' benchmarks/Cargo.toml deny.toml
}

python_changed() {
  changed_since_base \
    .github/workflows/ci.yml \
    .github/workflows/python.yml \
    Cargo.lock \
    Cargo.toml \
    deny.toml \
    crates/rusty-modbus-client \
    crates/rusty-modbus-codec \
    crates/rusty-modbus-frame \
    crates/rusty-modbus-python \
    crates/rusty-modbus-server \
    crates/rusty-modbus-tcp \
    crates/rusty-modbus-tls \
    crates/rusty-modbus-types
}

run_python_clippy() {
  (
    cd crates/rusty-modbus-python
    run cargo clippy --all-targets --locked -- -D warnings
  )
}

run cargo fmt --all --check
run rust-analyzer --version
run cargo clippy --workspace --all-targets --locked -- -D warnings
run scripts/test-local.sh

case "${RUSTY_MODBUS_RUN_PYTHON:-auto}" in
  always)
    run_python_clippy
    ;;
  never)
    echo
    echo "==> python clippy skipped (RUSTY_MODBUS_RUN_PYTHON=never)"
    ;;
  auto)
    if python_changed; then
      run_python_clippy
    else
      echo
      echo "==> python clippy skipped (no python-binding relevant changes)"
    fi
    ;;
  *)
    echo "RUSTY_MODBUS_RUN_PYTHON must be auto, always, or never" >&2
    exit 2
    ;;
esac

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
