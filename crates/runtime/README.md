# Fluentbase Runtime

The execution environment for rWASM contracts. It provides the host functions
exposed to smart contracts and manages context such as storage, account data and
precompiled contract calls.

Features:

- Executes rWASM bytecode with deterministic gas accounting
- Interfaces with the EVM and SVM compatibility layers
- Supports running in `no_std` mode for proofs
- Optional integration with Wasmtime for debugging (`wasmtime` feature)

The runtime is used by the testing framework and can also be embedded in other
Rust projects that wish to execute Fluentbase contracts.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
