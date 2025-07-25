# Fluentbase SDK

The main library for developing contracts that run on the Fluentbase runtime. It
provides the core `SharedAPI` trait, entrypoint macros and a collection of types
used by contracts.

Key components:

- `basic_entrypoint!` macro which expands into the WASM entrypoint
- `SharedAPI` trait offering read/write access to storage and context
- `fluentbase-sdk-derive` macros for reducing boilerplate
- Re-export of common primitive types from `fluentbase-types`

Add this crate to your contract's `Cargo.toml` together with
`fluentbase-sdk-derive` and call `basic_entrypoint!` with your contract type.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
