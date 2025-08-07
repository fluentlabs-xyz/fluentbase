# Fluentbase Build

This crate contains helper functions and a small CLI used to compile Fluentbase
smart contracts. It powers the build scripts for precompiled contracts and can
be invoked directly from a custom `build.rs`.

`build` supports deterministic Docker builds, custom Rust toolchains and
multiple output formats (WAT, rWASM, ABI, Solidity interface files and metadata).
Configuration is provided via the `BuildArgs` structure.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
