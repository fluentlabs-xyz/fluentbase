# Fluentbase SDK Derive

A set of procedural macros that generate boilerplate required by contracts using
the Fluentbase SDK. The most important macro is `#[derive(Contract)]` which
expands a struct into the expected entrypoint and dispatch logic.

Additional helper macros are available for generating error types and for
including a contract's own compiled rWASM for testing purposes.

See [`src/lib.rs`](src/lib.rs) for usage examples and a description of the
available attributes.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
