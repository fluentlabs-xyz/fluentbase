#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_codec::Encoder;
use fluentbase_sdk::{ContextReader, ContractInput, ExecutionContext, LowLevelAPI, LowLevelSDK};

// Function to deploy the contract
#[cfg(not(feature = "std"))]
#[no_mangle]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn deploy() {}

// Main function
#[cfg(not(feature = "std"))]
#[no_mangle]
#[cfg(target_arch = "wasm32")]
pub extern "C" fn main() {
    let ctx = ExecutionContext::default();
    let contract_input_struct = ContractInput {
        journal_checkpoint: ctx.journal_checkpoint(),
        contract_input: ctx.contract_input(),
        block_chain_id: ctx.block_chain_id(),
        contract_gas_limit: ctx.contract_gas_limit(),
        contract_address: ctx.contract_address(),
        contract_caller: ctx.contract_caller(),
        contract_value: ctx.contract_value(),
        contract_is_static: ctx.contract_is_static(),
        block_coinbase: ctx.block_coinbase(),
        block_timestamp: ctx.block_timestamp(),
        block_number: ctx.block_number(),
        block_difficulty: ctx.block_difficulty(),
        block_gas_limit: ctx.block_gas_limit(),
        block_base_fee: ctx.block_base_fee(),
        tx_gas_limit: ctx.tx_gas_limit(),
        tx_nonce: ctx.tx_nonce(),
        tx_gas_price: ctx.tx_gas_price(),
        tx_gas_priority_fee: ctx.tx_gas_priority_fee(),
        tx_caller: ctx.tx_caller(),
        tx_access_list: ctx.tx_access_list(),
        tx_blob_hashes: ctx.tx_blob_hashes(),
        tx_max_fee_per_blob_gas: ctx.tx_max_fee_per_blob_gas(),
    };
    LowLevelSDK::sys_write(&contract_input_struct.encode_to_vec(0));
}
