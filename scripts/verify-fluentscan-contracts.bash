#!/usr/bin/env bash
set -euo pipefail

FLUENTSCAN_API_BASE_URL="${FLUENTSCAN_API_BASE_URL:-https://api.fluentscan.xyz}"
GENESIS_VERSION="${GENESIS_VERSION:-v1.2.0}"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-1.93.1}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRY_RUN="${DRY_RUN:-false}"

echo "Fluentscan API: $FLUENTSCAN_API_BASE_URL"
echo "Genesis: $GENESIS_VERSION"
echo "Rust toolchain: $RUST_TOOLCHAIN"

require_command() {
  local command_name="$1"

  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "error: required command not found: $command_name" >&2
    exit 1
  fi
}

require_command python3

verify() {
  local address="$1"
  local name="$2"
  local manifest_path="$3"
  local manifest_abs="${REPO_ROOT}/${manifest_path}"
  local abi_path
  local payload

  echo "Verifying $name at $address"
  if [[ ! -f "$manifest_abs" ]]; then
    echo "error: manifest does not exist: $manifest_path" >&2
    exit 1
  fi
  abi_path="$(dirname "$manifest_abs")/abi.json"
  if [[ ! -f "$abi_path" ]]; then
    echo "error: ABI file does not exist: $abi_path" >&2
    echo "Run scripts/generate-contract-abis.bash and commit the generated abi.json files." >&2
    exit 1
  fi

  payload="$(CONTRACT_NAME="$name" \
    SDK_VERSION="$GENESIS_VERSION" \
    RUST_TOOLCHAIN="$RUST_TOOLCHAIN" \
    MANIFEST_PATH="$manifest_path" \
    COMMIT_REF="$GENESIS_VERSION" \
    ABI_PATH="$abi_path" \
    python3 - <<'PY'
import json
import os

with open(os.environ["ABI_PATH"], encoding="utf-8") as abi_file:
    abi = json.load(abi_file)

if not isinstance(abi, list):
    raise SystemExit("ABI must be a JSON array")

payload = {
    "contract_name": os.environ["CONTRACT_NAME"],
    "abi": abi,
    "compile_settings": {
        "sdk_version": os.environ["SDK_VERSION"],
        "no_default_features": True,
        "rust_toolchain": os.environ["RUST_TOOLCHAIN"],
        "manifest_path": os.environ["MANIFEST_PATH"],
    },
    "git_source": {
        "repository_url": "https://github.com/fluentlabs-xyz/fluentbase",
        "commit_ref": os.environ["COMMIT_REF"],
    },
}

print(json.dumps(payload, separators=(",", ":")))
PY
  )"

  if [[ "$DRY_RUN" == "true" ]]; then
    echo "$payload"
    echo
    return
  fi

  require_command curl
  curl -fsS "${FLUENTSCAN_API_BASE_URL}/api/v2/smart-contracts/${address}/verification/via/fluent" \
    -H 'content-type: application/json' \
    --data-raw "$payload"

  echo
}

verify "0x0000000000000000000000000000000000520001" "EVM Runtime" "contracts/evm/Cargo.toml"
verify "0x0000000000000000000000000000000000520005" "WebAuthn Verifier" "contracts/webauthn/Cargo.toml"
verify "0x0000000000000000000000000000000000520006" "OAuth2 Verifier" "contracts/oauth2/Cargo.toml"
verify "0x0000000000000000000000000000000000520007" "Nitro Verifier" "contracts/nitro/Cargo.toml"
verify "0x0000000000000000000000000000000000520008" "Universal Token Runtime" "contracts/universal-token/Cargo.toml"
verify "0x0000000000000000000000000000000000520009" "WASM Runtime" "contracts/wasm/Cargo.toml"
verify "0x0000000000000000000000000000000000520010" "Runtime Upgrade" "contracts/runtime-upgrade/Cargo.toml"
verify "0x0000000000000000000000000000000000520fee" "Fee Manager" "contracts/fee-manager/Cargo.toml"
verify "0x0000F90827F1C53a10cb7A02335B175320002935" "EIP-2935" "contracts/eip2935/Cargo.toml"
verify "0x0000000000000000000000000000000000000100" "EIP-7951" "contracts/eip7951/Cargo.toml"

verify "0x0000000000000000000000000000000000000001" "secp256k1 Recover" "contracts/ecrecover/Cargo.toml"
verify "0x0000000000000000000000000000000000000002" "SHA256" "contracts/sha256/Cargo.toml"
verify "0x0000000000000000000000000000000000000003" "RIPEMD160" "contracts/ripemd160/Cargo.toml"
verify "0x0000000000000000000000000000000000000004" "Identity" "contracts/identity/Cargo.toml"
verify "0x0000000000000000000000000000000000000005" "BigModExp" "contracts/modexp/Cargo.toml"
verify "0x0000000000000000000000000000000000000006" "BN256 Add" "contracts/bn256/Cargo.toml"
verify "0x0000000000000000000000000000000000000007" "BN256 Mul" "contracts/bn256/Cargo.toml"
verify "0x0000000000000000000000000000000000000008" "BN256 Pair" "contracts/bn256/Cargo.toml"
verify "0x0000000000000000000000000000000000000009" "BLAKE2F" "contracts/blake2f/Cargo.toml"
verify "0x000000000000000000000000000000000000000a" "KZG Point Evaluation" "contracts/kzg/Cargo.toml"
verify "0x000000000000000000000000000000000000000b" "BLS12-381 G1 Add" "contracts/bls12381/Cargo.toml"
verify "0x000000000000000000000000000000000000000c" "BLS12-381 G1 MSM" "contracts/bls12381/Cargo.toml"
verify "0x000000000000000000000000000000000000000d" "BLS12-381 G2 Add" "contracts/bls12381/Cargo.toml"
verify "0x000000000000000000000000000000000000000e" "BLS12-381 G2 MSM" "contracts/bls12381/Cargo.toml"
verify "0x000000000000000000000000000000000000000f" "BLS12-381 Pairing" "contracts/bls12381/Cargo.toml"
verify "0x0000000000000000000000000000000000000010" "BLS12-381 Map G1" "contracts/bls12381/Cargo.toml"
verify "0x0000000000000000000000000000000000000011" "BLS12-381 Map G2" "contracts/bls12381/Cargo.toml"
