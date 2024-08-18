#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

#[cfg(feature = "blended")]
mod blended;
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
#[cfg(feature = "svm")]
mod svm;
mod utils;
#[cfg(feature = "wasm")]
mod wasm;
