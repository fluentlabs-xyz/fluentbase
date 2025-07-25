# Fluentbase ERC20

A reference implementation of the ERC20 token standard built using the
Fluentbase SDK. It demonstrates the recommended project layout and how to use the
SDK entrypoint macros.

The contract exposes the usual `transfer`, `approve` and `transfer_from` methods
and stores balances in the Fluentbase key-value storage. The implementation is
`no_std` compatible and can be compiled to rWASM via the [`fluentbase-build`](../build)
crate.

This contract is included in the system precompile set for testing purposes.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
