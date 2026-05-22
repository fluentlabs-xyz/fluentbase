# Fluentbase SDK Testing

Testing utilities for Fluentbase contracts. This crate embeds the runtime and
provides helpers for executing rWASM modules in unit tests. It also re-exports
the `include_this_wasm!` macro which allows a contract to include its own WASM
binary during tests.

The EVM test harness is powered by a forked `revm` and can run Solidity contracts
or EVM compatibility precompiles directly.

## EVM transaction tests

Prefer `TxBuilder` for new EVM-style tests. It keeps transaction setup in one
place and returns assertion helpers that make expected outcomes explicit:

```rust
let tx = TxBuilder::call(&mut ctx, callee)
    .caller(caller)
    .input(calldata)
    .gas_limit(3_000_000)
    .execute();

tx.expect_ok().expect_gas_used(21_000);

let deploy = TxBuilder::create(&mut ctx, deployer, init_code).execute();
let contract = deploy.expect_ok().expect_expected_created_address().created_address();

let failure = TxBuilder::call(&mut ctx, callee)
    .caller(caller)
    .gas_limit(100_000)
    .execute();
failure
    .expect_halt()
    .expect_reason(HaltReason::OutOfGas)
    .expect_gas_used(100_000);
```

Existing helpers that return raw `ExecutionResult` can use `TxResultExt` for the
same `expect_*` assertion style.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
