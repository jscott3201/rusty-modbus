#!/usr/bin/env bash
# Run a Docker-only Modbus e2e smoke: server container + client containers.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

tag="${RUSTY_MODBUS_DOCKER_TAG:-rusty-modbus:local}"
network="rusty-modbus-smoke-$$"
server="rusty-modbus-server-$$"
port="${RUSTY_MODBUS_DOCKER_PORT:-5502}"

cleanup() {
  docker rm -f "$server" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker network create "$network" >/dev/null
docker run \
  --detach \
  --name "$server" \
  --network "$network" \
  "$tag" \
  --unit-id 1 \
  server \
  --listen "0.0.0.0:${port}" \
  --holding 0=100 >/dev/null

ready=0
for _ in $(seq 1 60); do
  if docker run --rm --network "$network" "$tag" \
    --host "$server" \
    --port "$port" \
    --unit-id 1 \
    --timeout 1 \
    read hr 0 1 >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.2
done

if [[ "$ready" != 1 ]]; then
  docker logs "$server" >&2 || true
  echo "server container did not become ready" >&2
  exit 1
fi

docker run --rm --network "$network" "$tag" \
  --host "$server" \
  --port "$port" \
  --unit-id 1 \
  write hr 0 48879 >/dev/null

output="$(docker run --rm --network "$network" "$tag" \
  --host "$server" \
  --port "$port" \
  --unit-id 1 \
  --format json \
  read hr 0 1)"

case "$output" in
  *48879*) ;;
  *)
    echo "unexpected read output:" >&2
    echo "$output" >&2
    exit 1
    ;;
esac

docker run --rm "$tag" --help >/dev/null
docker run --rm "$tag" dashboard --help >/dev/null

echo "docker smoke ok: $tag"
