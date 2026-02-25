# Fluentbase Contracts

This crate contains the system precompiled contracts that are bundled with the
Fluentbase runtimes. They expose compatibility layers for the EVM and SVM, along
with utilities such as hashing functions and token standards.

The build script compiles each contract to rWASM and embeds the resulting binaries
under `assets/`. These artifacts are later included in genesis files so that
clients can deploy them automatically.

Contracts include:

- EVM compatibility layer
- SVM (Solana VM) compatibility layer
- Standard cryptographic primitives (SHA256, Blake2, etc.)
- The reference ERC20 implementation

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
