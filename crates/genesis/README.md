# Fluentbase Genesis

Utilities for creating and manipulating genesis files that bundle Fluentbase
precompiled contracts. The resulting JSON genesis can be used by Reth or other
clients to start a local development network with the correct rWASM binaries
embedded.

The crate exposes helpers for loading the default devnet genesis as well as for
building custom configurations programmatically.

```rust
use fluentbase_genesis::devnet_genesis_from_file;
let genesis = devnet_genesis_from_file();
```

This crate is part of the [Fluentbase](https://github.com/fluentlabs-xyz/fluentbase) project.
