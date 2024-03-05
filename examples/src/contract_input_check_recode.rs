use crate::deploy_internal;
use fluentbase_codec::Encoder;
use fluentbase_sdk::evm::{ContractInput, ExecutionContext};
use fluentbase_types::ExitCode;
use revm_primitives::keccak256;

pub fn deploy() {
    deploy_internal(include_bytes!("../bin/contract_input_check_recode.wasm"))
}

pub fn main() {
    let ctx = ExecutionContext::default();

    let contract_input = ExecutionContext::contract_input();
    let contract_input_size = ExecutionContext::contract_input_size() as usize as u32;
    if contract_input_size != contract_input.len() as u32 {
        panic!("contract input size doesnt match")
    }
    let env_chain_id = ExecutionContext::env_chain_id();
    let contract_address = ExecutionContext::contract_address();
    let contract_caller = ExecutionContext::contract_caller();

    let contract_bytecode = ExecutionContext::contract_bytecode();
    let contract_code_size = ExecutionContext::contract_code_size();
    if contract_bytecode.len() != contract_code_size as usize {
        panic!("contract bytecode size doesnt match")
    }
    let contract_code_hash = ExecutionContext::contract_code_hash();
    if contract_code_hash.0 != keccak256(&contract_bytecode).0 {
        panic!("contract code hash doesnt match")
    }

    let journal_checkpoint = ExecutionContext::journal_checkpoint();

    let contract_value = ExecutionContext::contract_value();

    let contract_is_static = ExecutionContext::contract_is_static();

    let block_hash = ExecutionContext::block_hash();
    let block_coinbase = ExecutionContext::block_coinbase();
    let block_timestamp = ExecutionContext::block_timestamp();
    let block_number = ExecutionContext::block_number();
    let block_difficulty = ExecutionContext::block_difficulty();
    let block_gas_limit = ExecutionContext::block_gas_limit();
    let block_base_fee = ExecutionContext::block_base_fee();

    let tx_gas_price = ExecutionContext::tx_gas_price();
    let tx_gas_priority_fee = ExecutionContext::tx_gas_priority_fee();
    let tx_caller = ExecutionContext::tx_caller();

    let mut contract_input_struct = ContractInput {
        journal_checkpoint,
        contract_input: contract_input.clone(),
        contract_input_size,
        env_chain_id,
        contract_address,
        contract_caller,
        contract_bytecode,
        contract_code_size,
        contract_code_hash,
        contract_value,
        contract_is_static,
        block_hash,
        block_coinbase,
        block_timestamp,
        block_number,
        block_difficulty,
        block_gas_limit,
        block_base_fee,
        tx_gas_price,
        tx_gas_priority_fee,
        tx_caller,
    };
    ctx.fast_return_and_exit(
        contract_input_struct.encode_to_vec(0),
        ExitCode::Ok.into_i32(),
    );
}
