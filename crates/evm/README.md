# Fluentbase EVM

An execution engine implementing the Ethereum Virtual Machine semantics on top of
Fluentbase types. The crate provides a minimal EVM used for compatibility tests
and for running Solidity contracts compiled to rWASM.

Major components include:

- Opcode interpreter and gas accounting
- Memory model and stack implementation matching the Ethereum spec
- Helpers for translating rWASM runtime calls to EVM behavior

The engine is not intended to be feature complete but aims to be deterministic
and suitable for zero-knowledge proof generation.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
