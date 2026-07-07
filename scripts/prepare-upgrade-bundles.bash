#!/usr/bin/env bash

GENESIS_VERSION="${GENESIS_VERSION:-v1.3.0}"
RPC_URL="${RPC_URL:-https://rpc.testnet.fluent.xyz}"

CHAIN_ID=$(curl https://rpc.testnet.fluent.xyz -H 'content-type: application/json' --data-raw '{"id":1,"jsonrpc":"2.0","method":"eth_chainId","params":[]}' | jq -r '.result')
echo "Detected Chain ID: $CHAIN_ID"

mkdir -p ./target/bundles/${GENESIS_VERSION}-$CHAIN_ID

cargo build -p fluentbase-runtime-upgrade --release

make_bundle() {
  ./target/release/runtime-upgrade \
    --rpc ${RPC_URL} \
    --genesis ${GENESIS_VERSION} \
    --contract $1 \
    --safe-bundle ./target/bundles/${GENESIS_VERSION}-$CHAIN_ID/$1.json
}

make_bundle PRECOMPILE_EVM_RUNTIME
make_bundle PRECOMPILE_WEBAUTHN_VERIFIER
make_bundle PRECOMPILE_OAUTH2_VERIFIER
make_bundle PRECOMPILE_NITRO_VERIFIER
make_bundle PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME
make_bundle PRECOMPILE_WASM_RUNTIME
make_bundle PRECOMPILE_RUNTIME_UPGRADE
make_bundle PRECOMPILE_FEE_MANAGER
make_bundle PRECOMPILE_EIP2935
make_bundle PRECOMPILE_EIP7951

make_bundle PRECOMPILE_SECP256K1_RECOVER
make_bundle PRECOMPILE_SHA256
make_bundle PRECOMPILE_RIPEMD160
make_bundle PRECOMPILE_IDENTITY
make_bundle PRECOMPILE_BIG_MODEXP
make_bundle PRECOMPILE_BN256_ADD
make_bundle PRECOMPILE_BN256_MUL
make_bundle PRECOMPILE_BN256_PAIR
make_bundle PRECOMPILE_BLAKE2F
make_bundle PRECOMPILE_KZG_POINT_EVALUATION
make_bundle PRECOMPILE_BLS12_381_G1_ADD
make_bundle PRECOMPILE_BLS12_381_G1_MSM
make_bundle PRECOMPILE_BLS12_381_G2_ADD
make_bundle PRECOMPILE_BLS12_381_G2_MSM
make_bundle PRECOMPILE_BLS12_381_PAIRING
make_bundle PRECOMPILE_BLS12_381_MAP_G1
make_bundle PRECOMPILE_BLS12_381_MAP_G2
