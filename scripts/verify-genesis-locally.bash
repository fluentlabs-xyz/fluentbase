#!/usr/bin/env bash

set -euo pipefail

# Verification works only after v1.2.0-rc.1 version
TAG="${1:-v1.2.0-rc.1}"

docker build --platform=linux/amd64 -t fluentbase-build:local \
  --build-arg SDK_VERSION_TAG="${TAG}" \
  --build-arg RUST_TOOLCHAIN=1.92.0 .

FLUENTBASE_CONTRACTS_DOCKER=true \
FLUENTBASE_BUILD_DOCKER_IMAGE=fluentbase-build \
FLUENTBASE_BUILD_DOCKER_TAG=local \
FLUENTBASE_CONTRACTS_IGNORE_DEFAULT_RUST_FLAGS=true \
cargo b --release

TMP_DIR="$(mktemp -d)"
BASE_URL="https://github.com/fluentlabs-xyz/fluentbase/releases/download/${TAG}"

curl -fL "$BASE_URL/genesis-mainnet-${TAG}.json.gz" -o "$TMP_DIR/remote-mainnet.json.gz"
gunzip -c "$TMP_DIR/remote-mainnet.json.gz" > "$TMP_DIR/remote-mainnet.json"

REMOTE_MAINNET_SHA="$(sha256sum $TMP_DIR/remote-mainnet.json | awk '{print $1}')"
LOCAL_MAINNET_SHA="$(sha256sum ./crates/genesis/genesis-mainnet.json | awk '{print $1}')"

echo "Remote Genesis Hash: $REMOTE_MAINNET_SHA"
echo "Local Genesis Hash: $LOCAL_MAINNET_SHA"

test "$REMOTE_MAINNET_SHA" = "$LOCAL_MAINNET_SHA"
echo "OK: genesis hashes match"