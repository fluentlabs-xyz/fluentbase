# Fluentbase SVM

An implementation of the Solana Virtual Machine adapted for Fluentbase. The
runtime is used to run Solana-style programs and provides an alternative execution
environment alongside the EVM.

The crate includes instruction processors, loader logic and account handling
compatible with the Solana model. It is primarily used internally for testing
Solana interoperability with rWASM.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
