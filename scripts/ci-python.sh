#!/usr/bin/env bash
# Local mirror of .github/workflows/python.yml for the workspace-excluded PyO3
# bindings. Uses uv so contributors do not need a persistent Python virtualenv.
#
# Usage:
#   scripts/ci-python.sh
#   RUSTY_MODBUS_PYTHON=3.14t scripts/ci-python.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

crate_dir="$PWD/crates/rusty-modbus-python"
python_request="${RUSTY_MODBUS_PYTHON:-3.14}"
maturin_req="${RUSTY_MODBUS_MATURIN:-maturin>=1.12,<2}"
dist_dir="${RUSTY_MODBUS_PYTHON_DIST:-dist-local}"

if [[ "$dist_dir" != /* ]]; then
  dist_dir="$crate_dir/$dist_dir"
fi

case "$dist_dir" in
  "$crate_dir"/dist*) ;;
  *)
    echo "RUSTY_MODBUS_PYTHON_DIST must point under $crate_dir/dist*" >&2
    exit 2
    ;;
esac

case "$python_request" in
  *t*) expected_tag="${RUSTY_MODBUS_PYTHON_EXPECTED_TAG:-cp314t}" ;;
  *) expected_tag="${RUSTY_MODBUS_PYTHON_EXPECTED_TAG:-cp314-}" ;;
esac

run() {
  echo
  echo "==> $*"
  "$@"
}

if ! command -v uv >/dev/null 2>&1; then
  echo "uv is required for scripts/ci-python.sh" >&2
  exit 127
fi

cd "$crate_dir"

rm -rf "$dist_dir"
mkdir -p "$dist_dir"

python_bin="${RUSTY_MODBUS_PYTHON_BIN:-}"
if [[ -z "$python_bin" ]]; then
  python_bin="$(uv python find --no-project --system "$python_request")"
fi

uv_args=(--no-project --python "$python_bin")

run uv run "${uv_args[@]}" --with "$maturin_req" \
  maturin build --release --locked -o "$dist_dir" -i "$python_bin"

shopt -s nullglob
wheels=("$dist_dir"/*.whl)
shopt -u nullglob

if (( ${#wheels[@]} == 0 )); then
  echo "no wheels were built in $dist_dir" >&2
  exit 1
fi

wheel=""
for candidate in "${wheels[@]}"; do
  if [[ "$(basename "$candidate")" == *"$expected_tag"* ]]; then
    wheel="$candidate"
    break
  fi
done

if [[ -z "$wheel" ]]; then
  echo "expected a wheel tag containing $expected_tag, found:" >&2
  printf '  %s\n' "${wheels[@]}" >&2
  exit 1
fi

if [[ "${RUSTY_MODBUS_PYTHON_GIL:-}" == "0" || "$python_request" == *t* ]]; then
  run env PYTHON_GIL=0 uv run "${uv_args[@]}" --with "$wheel" --with pytest --with pytest-asyncio \
    python -m pytest tests/ -v
else
  run uv run "${uv_args[@]}" --with "$wheel" --with pytest --with pytest-asyncio \
    python -m pytest tests/ -v
fi

run uv run "${uv_args[@]}" --with "$wheel" --with mypy \
  python -m mypy.stubtest rusty_modbus --allowlist stubtest-allowlist.txt --ignore-unused-allowlist

run uv run "${uv_args[@]}" --with "$wheel" --with pyright \
  python -m pyright --verifytypes rusty_modbus
