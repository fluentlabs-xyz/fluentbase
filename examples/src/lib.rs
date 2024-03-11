#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloc::vec::Vec;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[cfg(feature = "cairo")]
mod cairo;
#[cfg(feature = "contract_input_check_recode")]
mod contract_input_check_recode;
#[cfg(feature = "erc20")]
mod erc20;
#[cfg(feature = "evm_call_from_wasm")]
mod evm_call_from_wasm;
#[cfg(feature = "greeting")]
mod greeting;
#[cfg(feature = "keccak256")]
mod keccak256;
#[cfg(feature = "panic")]
mod panic;
#[cfg(feature = "poseidon")]
mod poseidon;
#[cfg(feature = "rwasm")]
mod rwasm;
#[cfg(feature = "secp256k1")]
mod secp256k1;
#[cfg(feature = "stack")]
mod stack;

macro_rules! export_and_forward {
    ($fn_name:ident) => {
        #[cfg(not(feature = "std"))]
        #[no_mangle]
        #[cfg(target_arch = "wasm32")]
        pub extern "C" fn $fn_name() {
            #[cfg(feature = "cairo")]
            cairo::$fn_name();
            #[cfg(feature = "erc20")]
            erc20::$fn_name();
            #[cfg(feature = "greeting")]
            greeting::$fn_name();
            #[cfg(feature = "contract_input_check_recode")]
            contract_input_check_recode::$fn_name();
            #[cfg(feature = "evm_call_from_wasm")]
            evm_call_from_wasm::$fn_name();
            #[cfg(feature = "keccak256")]
            keccak256::$fn_name();
            #[cfg(feature = "poseidon")]
            poseidon::$fn_name();
            #[cfg(feature = "secp256k1")]
            secp256k1::$fn_name();
            #[cfg(feature = "panic")]
            panic::$fn_name();
            #[cfg(feature = "rwasm")]
            rwasm::$fn_name();
            #[cfg(feature = "stack")]
            stack::$fn_name();
        }
    };
}

export_and_forward!(deploy);
export_and_forward!(main);

pub(crate) fn deploy_internal<const N: usize>(bytes: &'static [u8; N]) {
    LowLevelSDK::sys_write(bytes);
    LowLevelSDK::sys_halt(0);
}

pub(crate) fn get_input() -> Vec<u8> {
    let input_size = LowLevelSDK::sys_input_size();
    let mut input_buffer = Vec::with_capacity(input_size as usize);
    LowLevelSDK::sys_read(&mut input_buffer, 0);
    input_buffer
}

pub(crate) fn write_output(output: Vec<u8>) {
    LowLevelSDK::sys_write(&output);
    LowLevelSDK::sys_halt(0);
}
