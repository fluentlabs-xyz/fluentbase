extern crate fluentbase_sdk;

use core::marker::PhantomData;
use fluentbase_sdk::{alloc_slice, basic_entrypoint, Bytes, ExitCode, LowLevelSDK, SharedAPI};
use revm_precompile::{PrecompileError, PrecompileResult};

pub trait PrecompileInvokeFunc {
    fn call(input: &Bytes, gas: u64) -> PrecompileResult;
}

macro_rules! define_precompile_func {
    ($name:ident, $path:expr) => {
        #[derive(Default)]
        pub struct $name;

        impl PrecompileInvokeFunc for $name {
            fn call(input: &Bytes, gas: u64) -> PrecompileResult {
                $path(input, gas)
            }
        }
    };
}

define_precompile_func!(BlakeInvokeFunc, revm_precompile::blake2::run);
// TODO(dmitry123): "add BN128 functions (mul, add, pair)"
define_precompile_func!(Sha256InvokeFunc, revm_precompile::hash::sha256_run);
define_precompile_func!(Ripemd160InvokeFunc, revm_precompile::hash::ripemd160_run);
define_precompile_func!(IdentityInvokeFunc, revm_precompile::identity::identity_run);
// TODO(dmitry123): "add KZG functions"
define_precompile_func!(ModexpInvokeFunc, revm_precompile::modexp::berlin_run);
define_precompile_func!(
    EcrecoverInvokeFunc,
    revm_precompile::secp256k1::ec_recover_run
);

#[derive(Default)]
pub struct PRECOMPILE<FN: PrecompileInvokeFunc> {
    _pd: PhantomData<FN>,
}

impl<FN: PrecompileInvokeFunc> PRECOMPILE<FN> {
    pub fn deploy<SDK: SharedAPI>(&self) {}

    pub fn main<SDK: SharedAPI>(&self) {
        let input_size = LowLevelSDK::input_size();
        let input = alloc_slice(input_size as usize);
        LowLevelSDK::read(input.as_mut_ptr(), input_size, 0);
        let input = Bytes::copy_from_slice(input);
        let (_gas_used, return_bytes) = FN::call(&input, u64::MAX).unwrap_or_else(|err| {
            SDK::exit(map_precompile_error(err).into_i32());
        });
        LowLevelSDK::write(return_bytes.as_ptr(), return_bytes.len() as u32);
    }
}

pub(crate) fn map_precompile_error(err: PrecompileError) -> ExitCode {
    match err {
        PrecompileError::OutOfGas => ExitCode::OutOfFuel,
        PrecompileError::Blake2WrongLength => ExitCode::PrecompileError,
        PrecompileError::Blake2WrongFinalIndicatorFlag => ExitCode::PrecompileError,
        PrecompileError::ModexpExpOverflow => ExitCode::PrecompileError,
        PrecompileError::ModexpBaseOverflow => ExitCode::PrecompileError,
        PrecompileError::ModexpModOverflow => ExitCode::PrecompileError,
        PrecompileError::Bn128FieldPointNotAMember => ExitCode::PrecompileError,
        PrecompileError::Bn128AffineGFailedToCreate => ExitCode::PrecompileError,
        PrecompileError::Bn128PairLength => ExitCode::PrecompileError,
        PrecompileError::BlobInvalidInputLength => ExitCode::PrecompileError,
        PrecompileError::BlobMismatchedVersion => ExitCode::PrecompileError,
        PrecompileError::BlobVerifyKzgProofFailed => ExitCode::PrecompileError,
        PrecompileError::Other(_) => ExitCode::PrecompileError,
    }
}

#[cfg(feature = "blake2")]
basic_entrypoint!(PRECOMPILE<BlakeInvokeFunc>);
#[cfg(feature = "sha256")]
basic_entrypoint!(PRECOMPILE<Sha256InvokeFunc>);
#[cfg(feature = "ripemd160")]
basic_entrypoint!(PRECOMPILE<Ripemd160InvokeFunc>);
#[cfg(feature = "identity")]
basic_entrypoint!(PRECOMPILE<IdentityInvokeFunc>);
#[cfg(feature = "modexp")]
basic_entrypoint!(PRECOMPILE<ModexpInvokeFunc>);
#[cfg(feature = "ecrecover")]
basic_entrypoint!(PRECOMPILE<EcrecoverInvokeFunc>);
