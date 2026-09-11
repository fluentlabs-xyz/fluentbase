#!/usr/bin/env bash
set -euo pipefail

RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-1.93.1}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target/contracts}"

require_command() {
  local command_name="$1"

  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 1
  fi
}

require_command cargo
require_command mktemp
require_command python3

echo "Rust toolchain: $RUST_TOOLCHAIN"
echo "Cargo target dir: $CARGO_TARGET_DIR"

find "$REPO_ROOT/contracts" -mindepth 2 -maxdepth 2 -name Cargo.toml | sort | while read -r manifest_path; do
  contract_dir="$(dirname "$manifest_path")"
  contract_name="$(basename "$contract_dir")"
  tmp_dir="$(mktemp -d)"
  abi_path="${contract_dir}/abi.json"

  echo "Generating ABI for contracts/${contract_name}"

  if (
    cd "$contract_dir"
    export CARGO_TARGET_DIR
    cargo run \
      --manifest-path "${REPO_ROOT}/Cargo.toml" \
      -p fluentbase-build \
      -- \
      --rust-version "$RUST_TOOLCHAIN" \
      --no-default-features \
      --generate abi \
      --output-path "$tmp_dir"
  ); then
    if [[ ! -s "${tmp_dir}/abi.json" ]]; then
      printf '[]\n' >"${tmp_dir}/abi.json"
    fi
  else
    echo "warning: failed to generate ABI for contracts/${contract_name}; writing empty ABI" >&2
    printf '[]\n' >"${tmp_dir}/abi.json"
  fi

  ABI_INPUT="${tmp_dir}/abi.json" ABI_OUTPUT="$abi_path" CONTRACT_NAME="$contract_name" python3 - <<'PY'
import json
import os

with open(os.environ["ABI_INPUT"], encoding="utf-8") as abi_file:
    abi = json.load(abi_file)

if not isinstance(abi, list):
    raise SystemExit("generated ABI must be a JSON array")

# A contract without a `#[router]` (a hand-rolled entrypoint such as webauthn) generates nothing;
# its published ABI is maintained by hand and must not be replaced with an empty array.
output_path = os.environ["ABI_OUTPUT"]
if not abi and os.path.exists(output_path):
    with open(output_path, encoding="utf-8") as existing_file:
        existing = json.load(existing_file)
    if existing:
        print(f"contracts/{os.environ['CONTRACT_NAME']}: no router found, keeping the hand-maintained ABI")
        raise SystemExit(0)

with open(output_path, "w", encoding="utf-8") as abi_file:
    json.dump(abi, abi_file, indent=2)
    abi_file.write("\n")
PY

  rm -rf "$tmp_dir"
done
