//! Cryptographic primitives and runtime adapters used by Fluentbase contracts and host runtimes.
#![cfg_attr(not(feature = "std"), no_std)]
/// This library is copied from SP1 (sp1/crates/zkvm/lib/Cargo.toml),
/// but system builtins are replaced with Fluentbase
extern crate alloc;
extern crate core;

pub mod bls12381;
pub mod bn254;
pub mod ecdsa;
pub mod ed25519;
mod ristretto255;
pub mod secp256k1;
pub mod secp256r1;
mod sha256;
pub use sha256::*;
mod blake3;
mod keccak256;
pub use keccak256::*;
pub mod utils;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "wasm32")] {
        pub type CryptoRuntime = fluentbase_types::RwasmContext;
    } else if #[cfg(feature = "std")] {
        pub type CryptoRuntime = fluentbase_runtime::RuntimeContextWrapper;
    } else {
        compile_error!("fluentbase-crypto can't be used in this mode");
    }
}

#[cfg(target_endian = "big")]
compile_error!("fluentbase-crypto is not implemented for big-endian targets");
