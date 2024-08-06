#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;

#[cfg(feature = "blended")]
mod blended;
#[cfg(feature = "evm")]
mod evm;
#[cfg(feature = "evm_deployer")]
mod evm_deployer;
#[cfg(feature = "evm_loader")]
mod evm_loader;
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
#[cfg(feature = "wasm_deployer")]
mod wasm_deployer;
#[cfg(feature = "wasm_loader")]
mod wasm_loader;
