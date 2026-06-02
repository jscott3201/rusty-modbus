#!/usr/bin/env bash
# Build and validate local Docker runtime, distroless, and benchmark images.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

runtime_tag="${RUSTY_MODBUS_DOCKER_RUNTIME_TAG:-rusty-modbus:local}"
distroless_tag="${RUSTY_MODBUS_DOCKER_DISTROLESS_TAG:-rusty-modbus:distroless}"
bench_tag="${RUSTY_MODBUS_DOCKER_BENCH_TAG:-rusty-modbus-bench:distroless}"
bench_target="${RUSTY_MODBUS_DOCKER_BENCH_TARGET:-benchmark-distroless}"
bench_duration="${RUSTY_MODBUS_DOCKER_BENCH_DURATION:-1}"
bench_in_flight="${RUSTY_MODBUS_DOCKER_BENCH_IN_FLIGHT:-2}"

RUSTY_MODBUS_DOCKER_TAG="$runtime_tag" \
RUSTY_MODBUS_DOCKER_TARGET=runtime \
  scripts/docker-build.sh
RUSTY_MODBUS_DOCKER_TAG="$runtime_tag" \
  scripts/docker-smoke.sh

RUSTY_MODBUS_DOCKER_TAG="$distroless_tag" \
RUSTY_MODBUS_DOCKER_TARGET=distroless \
  scripts/docker-build.sh
RUSTY_MODBUS_DOCKER_TAG="$distroless_tag" \
  scripts/docker-smoke.sh

RUSTY_MODBUS_DOCKER_BENCH_TAG="$bench_tag" \
RUSTY_MODBUS_DOCKER_TARGET="$bench_target" \
  scripts/docker-bench.sh \
    --duration "$bench_duration" \
    --warmup 0 \
    --clients 1 \
    --in-flight "$bench_in_flight" \
    --operation mixed \
    --json
