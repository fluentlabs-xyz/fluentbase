#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

#[cfg(feature = "evm")]
mod evm;
#[cfg(any(
    feature = "blake2",
    feature = "sha256",
    feature = "ripemd160",
    feature = "identity",
    feature = "modexp",
    feature = "ecrecover"
))]
mod precompile;

/// Native smart contracts
#[cfg(any(feature = "multicall"))]
mod precompiles;
