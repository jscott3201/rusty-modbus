#!/usr/bin/env bash
# Local workspace test gate. Mirrors the Rust test step in PR CI:
# nextest for normal tests, then cargo for doctests.
#
# Usage:
#   scripts/test-local.sh
#   RUSTY_MODBUS_NEXTEST_PROFILE=default scripts/test-local.sh
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

export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"

nextest_profile="${RUSTY_MODBUS_NEXTEST_PROFILE:-ci}"

if have cargo-nextest; then
  run cargo nextest run --workspace --locked --profile "$nextest_profile"
else
  echo
  echo "==> cargo-nextest missing; falling back to cargo test --workspace --locked"
  echo "==> install it for faster local runs: cargo install cargo-nextest --locked"
  run cargo test --workspace --locked
fi

run cargo test --workspace --locked --doc
