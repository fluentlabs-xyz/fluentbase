#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;
extern crate paste;

use fluentbase_codec::Encoder;
use fluentbase_sdk::LowLevelAPI;
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
        #[cfg(any(
            feature = "evm_loader_contract_entry",
            feature = "ecl_contract_entry",
            feature = "wasm_loader_contract_entry",
            feature = "wcl_contract_entry"
        ))]
        #[cfg(not(feature = "std"))]
        #[no_mangle]
        #[cfg(target_arch = "wasm32")]
        pub extern "C" fn $fn_name() {
            #[cfg(feature = "evm_loader_contract_entry")]
            contracts::evm_loader::$fn_name();
            #[cfg(feature = "ecl_contract_entry")]
            contracts::ecl::$fn_name();
            #[cfg(feature = "wasm_loader_contract_entry")]
            contracts::wasm_loader::$fn_name();
            #[cfg(feature = "wcl_contract_entry")]
            contracts::wcl::$fn_name();
        }
    };
}

export_and_forward!(deploy);
export_and_forward!(main);
