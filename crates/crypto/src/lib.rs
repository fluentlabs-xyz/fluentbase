///!
/// This library is copied from SP1 (sp1/crates/zkvm/lib/Cargo.toml),
/// but system builtins are replaced with Fluentbase
///
pub mod bls12381;
pub mod bn254;
pub mod ecdsa;
pub mod ed25519;
pub mod secp256k1;
pub mod secp256r1;
pub mod utils;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        pub type MathRuntime = fluentbase_runtime::RuntimeContextWrapper;
    } else {
        pub type MathRuntime = fluentbase_sdk::rwasm::RwasmContext;
    }
}

#[cfg(target_endian = "big")]
compile_error!("fluentbase-crypto is not implemented for big-endian targets");
