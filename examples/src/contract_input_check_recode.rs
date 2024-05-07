use fluentbase_codec::Encoder;
use fluentbase_sdk::{ContextReader, ContractInput, ExecutionContext};
use fluentbase_types::ExitCode;

pub fn deploy() {}

pub fn main() {
    let ctx = ExecutionContext::default();

    let contract_input = ExecutionContext::DEFAULT.contract_input();
    let block_chain_id = ExecutionContext::DEFAULT.block_chain_id();
    let contract_gas_limit = ExecutionContext::DEFAULT.contract_gas_limit();
    let contract_address = ExecutionContext::DEFAULT.contract_address();
    let contract_caller = ExecutionContext::DEFAULT.contract_caller();
    let journal_checkpoint = ExecutionContext::DEFAULT.journal_checkpoint();
    let contract_value = ExecutionContext::DEFAULT.contract_value();
    let contract_is_static = ExecutionContext::DEFAULT.contract_is_static();
    let block_coinbase = ExecutionContext::DEFAULT.block_coinbase();
    let block_timestamp = ExecutionContext::DEFAULT.block_timestamp();
    let block_number = ExecutionContext::DEFAULT.block_number();
    let block_difficulty = ExecutionContext::DEFAULT.block_difficulty();
    let block_gas_limit = ExecutionContext::DEFAULT.block_gas_limit();
    let block_base_fee = ExecutionContext::DEFAULT.block_base_fee();
    let tx_gas_price = ExecutionContext::DEFAULT.tx_gas_price();
    let tx_gas_priority_fee = ExecutionContext::DEFAULT.tx_gas_priority_fee();
    let tx_caller = ExecutionContext::DEFAULT.tx_caller();

    let contract_input_struct = ContractInput {
        journal_checkpoint,
        contract_input,
        block_chain_id,
        contract_gas_limit,
        contract_address,
        contract_caller,
        contract_value,
        contract_is_static,
        block_coinbase,
        block_timestamp,
        block_number,
        block_difficulty,
        block_gas_limit,
        block_base_fee,
        tx_gas_limit: ExecutionContext::DEFAULT.tx_gas_limit(),
        tx_nonce: ExecutionContext::DEFAULT.tx_nonce(),
        tx_gas_price,
        tx_gas_priority_fee,
        tx_caller,
        tx_access_list: ExecutionContext::DEFAULT.tx_access_list(),
    };
    ctx.fast_return_and_exit(
        contract_input_struct.encode_to_vec(0),
        ExitCode::Ok.into_i32(),
    );
}
