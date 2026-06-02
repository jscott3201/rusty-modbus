#!/usr/bin/env bash
# Build the local rusty-modbus container image.
#
# Usage:
#   scripts/docker-build.sh
#   RUSTY_MODBUS_DOCKER_TAG=rusty-modbus:dev scripts/docker-build.sh
#   RUSTY_MODBUS_DOCKER_TARGET=benchmark scripts/docker-build.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

tag="${RUSTY_MODBUS_DOCKER_TAG:-rusty-modbus:local}"
target="${RUSTY_MODBUS_DOCKER_TARGET:-runtime}"

platform_args=()
if [[ -n "${RUSTY_MODBUS_DOCKER_PLATFORM:-}" ]]; then
  platform_args=(--platform "${RUSTY_MODBUS_DOCKER_PLATFORM}")
fi

docker buildx build \
  --load \
  --target "$target" \
  --tag "$tag" \
  "${platform_args[@]}" \
  "$@" \
  .
