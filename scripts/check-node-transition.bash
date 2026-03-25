#!/usr/bin/env bash

set -euo pipefail

CHAIN="${1:-fluent-mainnet}"

case "$CHAIN" in
  fluent-mainnet)
    RPC_URL="https://rpc.fluent.xyz"
    DATA_DIR="./datadir/mainnet"
    ;;
  fluent-testnet)
    RPC_URL="https://rpc.testnet.fluent.xyz"
    DATA_DIR="./datadir/testnet"
    ;;
  fluent-devnet)
    RPC_URL="https://rpc.devnet.fluent.xyz"
    DATA_DIR="./datadir/devnet"
    ;;
  *)
    echo "Unknown chain: $CHAIN"
    echo "Usage: $0 [fluent-mainnet|fluent-testnet|fluent-devnet]"
    exit 1
    ;;
esac

mkdir -p "$DATA_DIR"

echo "Using chain: $CHAIN"
echo "Using RPC:   $RPC_URL"
echo "Using data:  $DATA_DIR"

echo "Building Fluent node in wasmtime mode..."
cargo b --release --manifest-path=../bins/fluent/Cargo.toml 2> build.log

echo "Killing any existing Fluent processes..."
pkill -INT -f fluent || true
sleep 5
if pgrep -f fluent > /dev/null; then
  echo "Force killing stuck Fluent..."
  pkill -9 -f fluent || true
fi

echo "Starting Fluent node (run 'tail -f ./reth.log' for logs)"
../target/release/fluent node --chain="$CHAIN" --datadir="$DATA_DIR" --http &> reth.log &
RETH_PID=$!

echo "Fluent started (PID $RETH_PID)"

get_block_number() {
  local url="$1"
  curl -s -X POST -H "Content-Type: application/json" \
    --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
    "$url" | jq -r '.result'
}

hex_to_dec() {
  local hex="$1"
  printf "%d\n" "$hex"
}

TIP_HEX="$(get_block_number "$RPC_URL")"
if [[ -z "$TIP_HEX" || "$TIP_HEX" == "null" ]]; then
  echo "Failed to fetch tip from $RPC_URL"
  kill "$RETH_PID" || true
  exit 1
fi

TIP_DEC="$(hex_to_dec "$TIP_HEX")"
echo "Waiting to reach chain tip... $TIP_DEC"

while true; do
  sleep 10

  CURRENT_HEX="$(get_block_number "http://127.0.0.1:8545")"

  if [[ -z "$CURRENT_HEX" || "$CURRENT_HEX" == "null" ]]; then
    echo "RPC not responding yet..."
    continue
  fi

  CURRENT_DEC="$(hex_to_dec "$CURRENT_HEX")"
  DIFF=$((TIP_DEC - CURRENT_DEC))

  echo "Current: $CURRENT_DEC  |  Tip: $TIP_DEC  |  Diff: $DIFF"

  if (( DIFF <= 1 )); then
    echo "Caught up (within $DIFF blocks) — shutting down fluent"
    kill "$RETH_PID"
    wait "$RETH_PID" || true
    break
  fi
done

echo "Synced to the latest known state"

echo "Re-executing node (wasmtime) from 1 block..."
../target/release/fluent re-execute --datadir="$DATA_DIR" --chain="$CHAIN" --from=1

echo "Rebuilding Fluent node in rwasm mode..."
cargo b --release \
  --no-default-features \
  --features=jemalloc,otlp,otlp-logs,reth-revm/portable,js-tracer,keccak-cache-global,asm-keccak,min-debug-logs,rocksdb \
  --manifest-path=../bins/fluent/Cargo.toml 2> build.log

echo "Re-executing node (rwasm) from 1 block..."
../target/release/fluent re-execute --datadir="$DATA_DIR" --chain="$CHAIN" --from=1