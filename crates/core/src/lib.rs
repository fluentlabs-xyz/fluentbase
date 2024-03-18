#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;
extern crate paste;

pub use fluentbase_types::ExitCode;

pub mod account;
pub mod account_types;
pub mod evm;

mod consts;
mod contracts;
pub mod fluent_host;
pub mod helpers;
mod utils;
pub mod wasm;

macro_rules! export_and_forward {
    ($fn_name:ident) => {
        #[cfg(any(feature = "evm_loader", feature = "ecl", feature = "wcl"))]
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
        }
    };
}

export_and_forward!(deploy);
export_and_forward!(main);
