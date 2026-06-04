# evm-e2e

`evm-e2e` runs Ethereum state-test shaped fixtures against Fluent EVM. It can also
generate and replay single Ethereum transaction fixtures for mainnet compatibility
checks.

## Generate a replay fixture

Use an RPC endpoint that supports `debug_traceTransaction` with `prestateTracer`:

```sh
node gen_fixture.mjs \
  --rpc-url "$RPC_URL" \
  --tx 0x... \
  --out fixtures/mainnet/tx-0x....json
```

Large bytecode values are externalized under `fixtures/reusable-bytecode/` and are
resolved automatically by the Fluent runner.

## Replay fixtures on Fluent EVM

```sh
cargo run --bin eth-replay -- fixtures/mainnet
```

Useful options:

- `--compare-reference` also runs the native EVM path and compares it with Fluent.
- `--report-jsonl replay.jsonl` writes one JSON record per fixture.
- `--trace` enables the trace inspector.
- `--fail-fast` stops after the first failing fixture.

Replay fixtures may include `config.chainid`; when present, Fluent execution uses
that value instead of the local devnet/testnet path fallback.
