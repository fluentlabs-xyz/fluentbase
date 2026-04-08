# Fluentbase Contracts

This crate contains the system precompiled contracts that are bundled with the
Fluentbase runtimes. They expose compatibility layers for the EVM and SVM, along
with utilities such as hashing functions and token standards.

The build script compiles contract WASM artifacts and embeds the resulting binaries
for genesis assembly. In CI/release flows this script can run deterministically
through `fluentbase-build` Docker tooling.

Contracts include:

- EVM compatibility layer
- SVM (Solana VM) compatibility layer
- Standard cryptographic primitives (SHA256, Blake2, etc.)
- The reference ERC20 implementation

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
