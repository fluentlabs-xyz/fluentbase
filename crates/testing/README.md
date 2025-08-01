# Fluentbase SDK Testing

Testing utilities for Fluentbase contracts. This crate embeds the runtime and
provides helpers for executing rWASM modules in unit tests. It also re-exports
the `include_this_wasm!` macro which allows a contract to include its own WASM
binary during tests.

The EVM test harness is powered by a forked `revm` and can run Solidity contracts
or EVM compatibility precompiles directly.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
