use alloc::vec;
use fluentbase_codec::Encoder;
use fluentbase_core::account::Account;
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{address, ExitCode, STATE_MAIN};
use hex_literal::hex;

pub fn deploy() {
    LowLevelSDK::sys_write(include_bytes!("../bin/evm_call_from_wasm.wasm"));
    LowLevelSDK::sys_halt(0);
}

pub fn main() {
    let ctx = ExecutionContext::default();

    // TODO call to evm
    // TODO should we pass evm_contract_address inside contract_input_data?

    // TODO must be evm_loader address
    let evm_loader_contract_address = address!("3927493649269146216491659123123997139871");
    // TODO codec encoded input so EVM contract could decode it and reroute to ECL?
    let evm_method = hex!("45773e4e").as_slice();

    // TODO 4test
    {
        if evm_loader_contract_address != ExecutionContext::contract_address() {
            panic!()
        }
        let contract_input_bytes = ExecutionContext::contract_input();
        if evm_method != contract_input_bytes.as_ref() {
            panic!()
        }
    }
    let contract_input = ContractInput {
        journal_checkpoint: ExecutionContext::journal_checkpoint().into(),
        contract_address: evm_loader_contract_address,
        contract_caller: ExecutionContext::contract_caller(),
        contract_input_size: evm_method.len() as u32,
        contract_input: evm_method.into(),
        // tx_caller: ExecutionContext::tx_caller(),
        ..Default::default()
    };
    let contract_input_vec = contract_input.encode_to_vec(0);
    let mut evm_call_result = vec![0u8; 96];
    let fuel: u32 = 10_000_000;
    let account = Account::new_from_jzkt(&evm_loader_contract_address);
    let bytecode = account.load_bytecode();
    if !bytecode.starts_with(&hex!("ef")) {
        panic!()
    }
    if bytecode.len() != 650124 {
        panic!()
    }
    LowLevelSDK::sys_exec(
        bytecode.as_ptr(),
        bytecode.len() as u32,
        contract_input_vec.as_ptr(),
        contract_input_vec.len() as u32,
        evm_call_result.as_mut_ptr(),
        evm_call_result.len() as u32,
        &fuel as *const u32,
        STATE_MAIN,
    );

    ctx.fast_return_and_exit(evm_call_result, ExitCode::Ok.into_i32());
}
