# Fluentbase Build

This crate contains helper functions and a small CLI used to compile Fluentbase
smart contracts. It powers the build scripts for precompiled contracts and can
be invoked directly from a custom `build.rs`.

`build` supports deterministic Docker builds, custom Rust toolchains and
multiple output formats (WAT, rWASM, ABI, Solidity interface files and metadata).
Configuration is provided via the `BuildArgs` structure.

```rust
use fluentbase_build::{build_with_args, BuildArgs, Artifact};

fn main() {
    build_with_args("./contracts/my_contract", BuildArgs {
        docker: true,
        generate: vec![Artifact::Rwasm, Artifact::Abi],
        ..Default::default()
    });
}
```

Artifacts are written to the `out/` directory by default. Set the
`FLUENTBASE_SKIP_BUILD` environment variable if you wish to skip compilation
(e.g., when cross compiling from another workspace).

See [`src/lib.rs`](src/lib.rs) for a full list of options and advanced usage
notes.

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
