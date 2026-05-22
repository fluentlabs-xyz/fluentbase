#!/usr/bin/env bash
set -euo pipefail

FLUENTSCAN_HOST="${FLUENTSCAN_HOST:-fluentscan.xyz}"
GENESIS_VERSION="${GENESIS_VERSION:-v1.2.0}"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-1.93.1}"

echo "Fluentscan: $FLUENTSCAN_HOST"
echo "Genesis: $GENESIS_VERSION"
echo "Rust toolchain: $RUST_TOOLCHAIN"

verify() {
  local address="$1"
  local name="$2"
  local manifest_path="$3"

  echo "Verifying $name at $address"

  curl -fsS "https://api.${FLUENTSCAN_HOST}/api/v2/smart-contracts/${address}/verification/via/fluent" \
    -H 'content-type: application/json' \
    --data-raw "{
      \"contract_name\":\"${name}\",
      \"abi\":[],
      \"compile_settings\":{
        \"sdk_version\":\"${GENESIS_VERSION}\",
        \"no_default_features\":true,
        \"rust_toolchain\":\"${RUST_TOOLCHAIN}\",
        \"manifest_path\":\"${manifest_path}\"
      },
      \"git_source\":{
        \"repository_url\":\"https://github.com/fluentlabs-xyz/fluentbase\",
        \"commit_ref\":\"${GENESIS_VERSION}\"
      }
    }"

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
verify "0x0000F90827F1C53a10cb7A02335B175320002935" "EIP-2935" "contracts/eip-2935/Cargo.toml"
verify "0x0000000000000000000000000000000000000100" "EIP-7951" "contracts/eip-7951/Cargo.toml"

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