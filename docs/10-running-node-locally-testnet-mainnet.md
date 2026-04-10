# Run a Fluent node locally (testnet and mainnet)

This guide shows the minimal commands to run a local Fluent node from this repository.

## Prerequisites

- Rust toolchain installed (project toolchain is pinned by `rust-toolchain`).
- `git` and build dependencies for Rust crates.
- Open ports for P2P and RPC if needed.

Build the node binary:

```bash
cd fluentbase
cargo build --bin fluent --release
```

Binary path used below:

```bash
./target/release/fluent
```

## Chain names

- `fluent-testnet` (chain id `20994`)
- `fluent-mainnet` (chain id `25363`)

Use separate data directories per network.

---

## Run local testnet node

Testnet should be bootstrapped from snapshot first.

```bash
mkdir -p ./datadir/testnet

./target/release/fluent init \
  --datadir=./datadir/testnet \
  --chain=fluent-testnet

./target/release/fluent download \
  --datadir=./datadir/testnet \
  --chain=fluent-testnet

./target/release/fluent node \
  --chain=fluent-testnet \
  --datadir=./datadir/testnet \
  --http
```

---

## Run local mainnet node

Mainnet can be started directly with its own datadir:

```bash
mkdir -p ./datadir/mainnet

./target/release/fluent node \
  --chain=fluent-mainnet \
  --datadir=./datadir/mainnet \
  --http
```

---

## Quick health check

With `--http` enabled, verify local RPC responds:

```bash
curl -s -X POST http://127.0.0.1:8545 \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

If it returns a hex block number (`0x...`), RPC is up.

## Notes

- Keep testnet and mainnet datadirs separate.
- First sync can take time and disk space.
- Stop node with `Ctrl+C` for clean shutdown.
