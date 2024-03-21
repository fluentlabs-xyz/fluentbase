#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;
extern crate paste;

pub use fluentbase_types::ExitCode;

mod account;
pub use account::*;
mod account_types;
pub use account_types::*;
#[cfg(feature = "ecl")]
pub mod evm;

pub mod consts;
mod contracts;
#[cfg(feature = "ecl")]
pub mod fluent_host;
pub mod helpers;
#[cfg(any(
    feature = "precompile_blake2",
    feature = "precompile_bn128",
    feature = "precompile_identity",
    feature = "precompile_kzg_point_evaluation",
    feature = "precompile_modexp",
    feature = "precompile_secp256k1",
))]
pub mod precompiles;
mod utils;
pub mod wasm;

macro_rules! export_and_forward {
    ($fn_name:ident) => {
        #[cfg(any(
            feature = "evm_loader",
            feature = "ecl",
            feature = "wcl",
            feature = "precompile_blake2",
            feature = "precompile_bn128",
            feature = "precompile_identity",
            feature = "precompile_kzg_point_evaluation",
            feature = "precompile_modexp",
            feature = "precompile_secp256k1",
        ))]
        #[cfg(not(feature = "std"))]
        #[no_mangle]
        #[cfg(target_arch = "wasm32")]
        pub extern "C" fn $fn_name() {
            #[cfg(feature = "evm_loader")]
            contracts::evm_loader::$fn_name();
            #[cfg(feature = "ecl")]
            contracts::ecl::$fn_name();
            #[cfg(feature = "wcl")]
            contracts::wcl::$fn_name();
            #[cfg(feature = "precompile_blake2")]
            precompiles::blake2::$fn_name();
            #[cfg(feature = "precompile_identity")]
            precompiles::identity::$fn_name();
            #[cfg(feature = "precompile_modexp")]
            precompiles::modexp::$fn_name();
            #[cfg(feature = "precompile_secp256k1")]
            precompiles::secp256k1::$fn_name();
        }
    };
}

export_and_forward!(deploy);
export_and_forward!(main);
