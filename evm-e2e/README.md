# evm-e2e

## FLU-28 Heavy EVM Memory Repros

The FLU-28 repro tests are ignored and require an explicit environment guard because they can
consume large amounts of memory. They cover both input families from the issue:

- the pinned upstream Ethereum `GeneralStateTests/stQuadraticComplexityTest` transactions
- the synthetic `MSTORE8` transaction that forces exactly 2 GiB of active EVM memory

First sync the pinned upstream Ethereum tests:

```bash
make -C evm-e2e sync_tests
```

Run the original quadratic-complexity transactions:

```bash
FLUENTBASE_RUN_HEAVY_EVM_REPRO=1 cargo test \
  --manifest-path evm-e2e/Cargo.toml \
  --release \
  --no-default-features \
  --features std,wasmtime \
  --package evm-e2e \
  heavy_tests::upstream_st_quadratic_complexity \
  -- --ignored --nocapture
```

Run the synthetic 2 GiB memory-expansion transaction with RSS reporting:

```bash
FLUENTBASE_RUN_HEAVY_EVM_REPRO=1 /usr/bin/time -v cargo test \
  --manifest-path evm-e2e/Cargo.toml \
  --release \
  --no-default-features \
  --features std,wasmtime \
  --package evm-e2e \
  heavy_tests::synthetic_memory::mstore8_2gib \
  -- --ignored --nocapture
```

For the synthetic fixture, the expected state root is zero so the runner records and compares
native EVM vs Fluent behavior without requiring a checked-in precomputed root for a machine-sized
allocation case.
