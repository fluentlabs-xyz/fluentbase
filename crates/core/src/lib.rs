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

#[cfg(feature = "evm_contract_entry")]
mod evm_contract_entry;
#[cfg(feature = "evm_loader_contract_entry")]
mod evm_loader_entry;
pub mod fluent_host;
mod utils;

macro_rules! export_and_forward {
    ($fn_name:ident) => {
        #[cfg(any(feature = "evm_loader_contract_entry", feature = "evm_contract_entry"))]
        #[cfg(not(feature = "std"))]
        #[no_mangle]
        #[cfg(target_arch = "wasm32")]
        pub extern "C" fn $fn_name() {
            #[cfg(feature = "evm_loader_contract_entry")]
            evm_loader_entry::$fn_name();
            #[cfg(feature = "evm_contract_entry")]
            evm_contract_entry::$fn_name();
        }
    };
}

export_and_forward!(deploy);
export_and_forward!(main);
