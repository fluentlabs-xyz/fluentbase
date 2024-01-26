#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]
#![feature(generic_const_exprs)]
extern crate fluentbase_sdk;

use fluentbase_codec::Encoder;
use fluentbase_sdk::{
    evm::{Bytes, ContractOutput, ContractOutputNoLogs},
    LowLevelAPI,
    LowLevelSDK,
};

#[cfg(feature = "erc20")]
mod erc20;
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
            #[cfg(feature = "erc20")]
            erc20::$fn_name();
            #[cfg(feature = "greeting")]
            greeting::$fn_name();
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

pub(crate) fn deploy_internal<const N: usize>(bytes: &'static [u8; N])
where
    [u8; N + ContractOutput::HEADER_SIZE]:,
{
    let contract_output = ContractOutputNoLogs {
        return_data: Bytes::from_static(bytes),
        logs: Default::default(),
    };
    let (buffer, length) =
        contract_output.encode_to_fixed::<{ N + ContractOutput::HEADER_SIZE }>(0);
    LowLevelSDK::sys_write(&buffer[..length]);
    LowLevelSDK::sys_halt(0);
}
