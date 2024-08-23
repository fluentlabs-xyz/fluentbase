#![allow(dead_code)]

extern crate fluentbase_sdk;

use core::marker::PhantomData;
use fluentbase_sdk::{alloc_slice, basic_entrypoint, Bytes, ExitCode, SharedAPI};
use revm_precompile::{PrecompileError, PrecompileErrors, PrecompileResult};

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

pub struct PRECOMPILE<SDK, FN: PrecompileInvokeFunc> {
    sdk: SDK,
    _pd: PhantomData<FN>,
}

impl<SDK: SharedAPI, FN: PrecompileInvokeFunc> PRECOMPILE<SDK, FN> {
    pub fn new(sdk: SDK) -> Self {
        Self {
            sdk,
            _pd: Default::default(),
        }
    }

    pub fn deploy(&self) {}

    pub fn main(&mut self) {
        let input_size = self.sdk.input_size();
        let input = alloc_slice(input_size as usize);
        self.sdk.read(input, 0);
        let input = Bytes::copy_from_slice(input);
        let call_output = FN::call(&input, self.sdk.fuel()).unwrap_or_else(|err| {
            self.sdk.exit(map_precompile_error(err).into_i32());
        });
        // self.sdk.charge_fuel(call_output.gas_used);
        let return_bytes = call_output.bytes;
        self.sdk.write(&return_bytes);
    }
}

pub(crate) fn map_precompile_error(err: PrecompileErrors) -> ExitCode {
    match err {
        PrecompileErrors::Error(err2) => match err2 {
            PrecompileError::OutOfGas => ExitCode::OutOfGas,
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
        },
        PrecompileErrors::Fatal { .. } => ExitCode::FatalExternalError,
    }
}

type BlakePrecompile<SDK> = PRECOMPILE<SDK, BlakeInvokeFunc>;
type Sha256Precompile<SDK> = PRECOMPILE<SDK, Sha256InvokeFunc>;
type Ripemd160Precompile<SDK> = PRECOMPILE<SDK, Ripemd160InvokeFunc>;
type IdentityPrecompile<SDK> = PRECOMPILE<SDK, IdentityInvokeFunc>;
type ModexpPrecompile<SDK> = PRECOMPILE<SDK, ModexpInvokeFunc>;
type EcrecoverPrecompile<SDK> = PRECOMPILE<SDK, EcrecoverInvokeFunc>;

#[cfg(feature = "blake2")]
basic_entrypoint!(BlakePrecompile);
#[cfg(feature = "sha256")]
basic_entrypoint!(Sha256Precompile);
#[cfg(feature = "ripemd160")]
basic_entrypoint!(Ripemd160Precompile);
#[cfg(feature = "identity")]
basic_entrypoint!(IdentityPrecompile);
#[cfg(feature = "modexp")]
basic_entrypoint!(ModexpPrecompile);
#[cfg(feature = "ecrecover")]
basic_entrypoint!(EcrecoverPrecompile);
