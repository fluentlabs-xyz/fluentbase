#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;

#[cfg(feature = "evm")]
mod evm;
#[cfg(any(
    feature = "blake2",
    feature = "sha256",
    feature = "ripemd160",
    feature = "identity",
    feature = "modexp",
    feature = "ecrecover",
))]
mod precompile;
