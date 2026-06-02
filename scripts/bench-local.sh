#!/usr/bin/env bash
# Local benchmark runner for repeatable performance checks.
#
# Usage:
#   scripts/bench-local.sh smoke
#   scripts/bench-local.sh codec --quick
#   scripts/bench-local.sh store --quick
#   scripts/bench-local.sh tcp --quick --noplot
#   scripts/bench-local.sh stress --duration 10 --clients 1 --in-flight 8 --json
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

mode="${1:-smoke}"
if [[ $# -gt 0 ]]; then
  shift
fi

profile_time="${RUSTY_MODBUS_BENCH_PROFILE_TIME:-1}"

run() {
  echo
  echo "==> $*"
  "$@"
}

case "$mode" in
  smoke)
    run cargo bench -p rusty-modbus-benchmarks --bench codec -- --noplot --profile-time "$profile_time"
    run cargo bench -p rusty-modbus-benchmarks --bench tcp_throughput tcp_pipelined -- --noplot --profile-time "$profile_time"
    ;;
  codec)
    run cargo bench -p rusty-modbus-benchmarks --bench codec -- "$@"
    ;;
  store)
    run cargo bench -p rusty-modbus-benchmarks --bench server_store -- "$@"
    ;;
  tcp)
    run cargo bench -p rusty-modbus-benchmarks --bench tcp_throughput -- "$@"
    ;;
  tcp-pipelined)
    run cargo bench -p rusty-modbus-benchmarks --bench tcp_throughput tcp_pipelined -- "$@"
    ;;
  stress)
    run cargo run --release -p rusty-modbus-benchmarks --bin stress-test -- "$@"
    ;;
  all)
    run cargo bench -p rusty-modbus-benchmarks -- "$@"
    ;;
  *)
    cat >&2 <<'USAGE'
Usage: scripts/bench-local.sh [smoke|codec|store|tcp|tcp-pipelined|stress|all] [args...]

Examples:
  scripts/bench-local.sh smoke
  scripts/bench-local.sh codec --quick --noplot
  scripts/bench-local.sh store --quick --noplot
  scripts/bench-local.sh tcp-pipelined --quick --noplot
  scripts/bench-local.sh stress --duration 10 --clients 1 --in-flight 8 --json
USAGE
    exit 2
    ;;
esac
