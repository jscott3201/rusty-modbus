#!/usr/bin/env bash
# Run comparable local and Docker stress benchmark matrices.
#
# Usage:
#   scripts/bench-suite.sh all
#   scripts/bench-suite.sh local --duration 10
#   scripts/bench-suite.sh docker --depths 1,8,16 --operations read,mixed
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

mode="${1:-all}"
case "$mode" in
  all | local | docker) shift || true ;;
  -h | --help)
    cat <<'USAGE'
Usage: scripts/bench-suite.sh [all|local|docker] [options]

Modes:
  all      Run local release, Alpine Docker, and distroless Docker matrices.
  local    Run only the local release matrix.
  docker   Run only Alpine Docker and distroless Docker matrices.

Options:
  --duration N      Measured seconds per row. Default: 5.
  --warmup N        Warmup seconds per row. Default: 1.
  --clients N       Client connections. Default: 1.
  --depths LIST     In-flight depths, comma or space separated. Default: 1,2,4,8,16.
  --operations LIST Operations, comma or space separated. Default: read,mixed.
  --output-dir DIR  Log output directory. Default: bench-output/stress-<timestamp>.
USAGE
    exit 0
    ;;
  --*) mode="all" ;;
  *)
    echo "unknown mode: $mode" >&2
    echo "use: scripts/bench-suite.sh [all|local|docker]" >&2
    exit 2
    ;;
esac

duration=5
warmup=1
clients=1
depths="1,2,4,8,16"
operations="read,mixed"
output_dir=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --duration)
      duration="$2"
      shift 2
      ;;
    --warmup)
      warmup="$2"
      shift 2
      ;;
    --clients)
      clients="$2"
      shift 2
      ;;
    --depths)
      depths="$2"
      shift 2
      ;;
    --operations)
      operations="$2"
      shift 2
      ;;
    --output-dir)
      output_dir="$2"
      shift 2
      ;;
    *)
      echo "unknown option: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$output_dir" ]]; then
  output_dir="bench-output/stress-$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$output_dir"

split_list() {
  local raw="${1//,/ }"
  read -r -a split_values <<<"$raw"
}

write_log() {
  local file="$1"
  shift
  echo "==> $*"
  "$@" 2>&1 | tee "$file"
}

run_local_matrix() {
  for operation in "${operation_values[@]}"; do
    for depth in "${depth_values[@]}"; do
      write_log \
        "$output_dir/local-${operation}-depth${depth}.log" \
        cargo run --release -p rusty-modbus-benchmarks --bin stress-test -- \
          --duration "$duration" \
          --warmup "$warmup" \
          --clients "$clients" \
          --in-flight "$depth" \
          --operation "$operation" \
          --json
    done
  done
}

build_docker_benchmarks() {
  RUSTY_MODBUS_DOCKER_TAG="${RUSTY_MODBUS_DOCKER_ALPINE_BENCH_TAG:-rusty-modbus-bench:alpine}" \
  RUSTY_MODBUS_DOCKER_TARGET=benchmark \
    scripts/docker-build.sh

  RUSTY_MODBUS_DOCKER_TAG="${RUSTY_MODBUS_DOCKER_DISTROLESS_BENCH_TAG:-rusty-modbus-bench:distroless}" \
  RUSTY_MODBUS_DOCKER_TARGET=benchmark-distroless \
    scripts/docker-build.sh
}

run_docker_matrix() {
  local label="$1"
  local tag="$2"

  for operation in "${operation_values[@]}"; do
    for depth in "${depth_values[@]}"; do
      write_log \
        "$output_dir/docker-${label}-${operation}-depth${depth}.log" \
        docker run --rm "$tag" \
          --duration "$duration" \
          --warmup "$warmup" \
          --clients "$clients" \
          --in-flight "$depth" \
          --operation "$operation" \
          --json
    done
  done
}

split_list "$depths"
depth_values=("${split_values[@]}")
split_list "$operations"
operation_values=("${split_values[@]}")

case "$mode" in
  all)
    run_local_matrix
    build_docker_benchmarks
    run_docker_matrix alpine "${RUSTY_MODBUS_DOCKER_ALPINE_BENCH_TAG:-rusty-modbus-bench:alpine}"
    run_docker_matrix distroless "${RUSTY_MODBUS_DOCKER_DISTROLESS_BENCH_TAG:-rusty-modbus-bench:distroless}"
    ;;
  local)
    run_local_matrix
    ;;
  docker)
    build_docker_benchmarks
    run_docker_matrix alpine "${RUSTY_MODBUS_DOCKER_ALPINE_BENCH_TAG:-rusty-modbus-bench:alpine}"
    run_docker_matrix distroless "${RUSTY_MODBUS_DOCKER_DISTROLESS_BENCH_TAG:-rusty-modbus-bench:distroless}"
    ;;
esac

echo "benchmark logs written to $output_dir"
