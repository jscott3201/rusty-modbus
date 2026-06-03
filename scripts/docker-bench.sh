#!/usr/bin/env bash
# Build and run the benchmark container target.
#
# Usage:
#   scripts/docker-bench.sh --duration 5 --clients 1 --in-flight 8 --json
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

tag="${RUSTY_MODBUS_DOCKER_BENCH_TAG:-rusty-modbus-bench:local}"
target="${RUSTY_MODBUS_DOCKER_TARGET:-benchmark}"

RUSTY_MODBUS_DOCKER_TAG="$tag" \
RUSTY_MODBUS_DOCKER_TARGET="$target" \
  scripts/docker-build.sh

docker run --rm "$tag" "$@"
