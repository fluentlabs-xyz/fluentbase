///!
/// This library is copied from SP1 (sp1/crates/zkvm/lib/Cargo.toml),
/// but system builtins are replaced with Fluentbase
///
mod bls12381;
mod bn254;
mod ecdsa;
mod ed25519;
mod secp256k1;
mod secp256r1;
mod utils;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        type MathRuntime = fluentbase_runtime::RuntimeContextWrapper;
    } else {
        type MathRuntime = fluentbase_sdk::rwasm::RwasmContext;
    }
}

#[cfg(target_endian = "big")]
compile_error!("fluentbase-crypto is not implemented for big-endian targets");
