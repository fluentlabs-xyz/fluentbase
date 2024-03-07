use crate::{
    assets::test_contracts::{
        CONTRACT_BYTECODE1,
        CONTRACT_BYTECODE1_METHOD_SAY_HELLO_WORLD_STR_ID,
    },
    core::testing_utils::TestingContext,
    test_helpers::wasm2rwasm,
};
use fluentbase_codec::Encoder;
use fluentbase_core::{account::Account, evm::calc_create_address};
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{
        EvmCallMethodInput,
        EvmCreateMethodInput,
        EVM_CALL_METHOD_ID,
        EVM_CREATE_METHOD_ID,
    },
};
use fluentbase_runtime::{
    types::{address, Address, Bytes, B256, U256},
    Runtime,
    RuntimeContext,
};
use hex_literal::hex;
use keccak_hash::keccak;

#[test]
fn test_create() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let expected_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let evm_contract_input_bytes = CONTRACT_BYTECODE1;

    let value = B256::left_padding_from(&hex!("1000"));
    let gas_limit: u32 = 10_000_000;
    let evm_create_method_input =
        EvmCreateMethodInput::new(value.0, evm_contract_input_bytes.to_vec(), gas_limit);
    let evm_create_core_input = CoreInput::new(
        EVM_CREATE_METHOD_ID,
        evm_create_method_input.encode_to_vec(0),
    );
    let evm_create_core_input_vec = evm_create_core_input.encode_to_vec(0);

    const IS_RUNTIME: bool = true;
    let wasm_binary = include_bytes!("../../../crates/core/bin/core.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary.as_slice(), false);
    let mut runtime_ctx = RuntimeContext::new(rwasm_binary);
    let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(true, Some(&mut runtime_ctx));
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
        .set_contract_input(Bytes::copy_from_slice(&evm_create_core_input_vec))
        .set_contract_input_size(evm_create_core_input_vec.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(expected_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE1))
        .set_contract_code_size(CONTRACT_BYTECODE1.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(CONTRACT_BYTECODE1).as_bytes()))
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx(Some(&mut runtime_ctx));

    let import_linker = Runtime::<()>::new_sovereign_linker();
    let mut output = test_ctx.run_rwasm_with_evm_input(runtime_ctx, &import_linker);

    assert_eq!(output.data().exit_code(), 0);
    assert_eq!(&expected_contract_address.to_vec(), output.data().output(),);
}

#[test]
fn test_call_after_create() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_nonce = 1;
    let caller_account = Account {
        address: caller_address,
        nonce: caller_nonce,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let expected_contract_address = calc_create_address(&caller_address, caller_nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let contract_value = U256::from_be_slice(&hex!("0123456789abcdef"));
    let contract_is_static = false;
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let evm_contract_input_bytes = CONTRACT_BYTECODE1;

    let create_value = B256::left_padding_from(&hex!("1000"));
    let call_value = B256::left_padding_from(&hex!("1000"));
    let gas_limit: u32 = 10_000_000;
    let evm_create_method_input =
        EvmCreateMethodInput::new(create_value.0, evm_contract_input_bytes.to_vec(), gas_limit);
    let evm_create_core_input = CoreInput::new(
        EVM_CREATE_METHOD_ID,
        evm_create_method_input.encode_to_vec(0),
    );
    let evm_create_core_input_vec = evm_create_core_input.encode_to_vec(0);

    let import_linker = Runtime::<()>::new_sovereign_linker();
    const IS_RUNTIME: bool = true;
    let wasm_binary = include_bytes!("../../../crates/core/bin/core.wasm");
    let rwasm_binary = wasm2rwasm(wasm_binary.as_slice(), false);
    let mut runtime_ctx = RuntimeContext::new(rwasm_binary.clone());
    let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(true, Some(&mut runtime_ctx));
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
        .set_contract_input(Bytes::copy_from_slice(&evm_create_core_input_vec))
        .set_contract_input_size(evm_create_core_input_vec.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(expected_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE1))
        .set_contract_code_size(CONTRACT_BYTECODE1.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(CONTRACT_BYTECODE1).as_bytes()))
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx(Some(&mut runtime_ctx));
    let mut output = test_ctx.run_rwasm_with_evm_input(runtime_ctx, &import_linker);
    let contract_address = Address::from_slice(output.data().output());

    assert_eq!(output.data().exit_code(), 0);
    assert_eq!(
        expected_contract_address.as_slice(),
        contract_address.as_slice()
    );

    let evm_call_method_input = EvmCallMethodInput::new(
        contract_address.into_array(),
        call_value.0,
        CONTRACT_BYTECODE1_METHOD_SAY_HELLO_WORLD_STR_ID.to_vec(),
        gas_limit,
    );
    let evm_call_core_input =
        CoreInput::new(EVM_CALL_METHOD_ID, evm_call_method_input.encode_to_vec(0));
    let evm_call_core_input_vec = evm_call_core_input.encode_to_vec(0);

    let mut runtime_ctx = RuntimeContext::new(rwasm_binary);
    test_ctx
        .contract_input_wrapper
        .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
        .set_contract_input(Bytes::copy_from_slice(&evm_create_core_input_vec))
        .set_contract_input_size(evm_create_core_input_vec.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_address(expected_contract_address)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE1))
        .set_contract_code_size(CONTRACT_BYTECODE1.len() as u32)
        .set_contract_code_hash(B256::from_slice(keccak(CONTRACT_BYTECODE1).as_bytes()))
        .set_contract_value(contract_value)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx(Some(&mut runtime_ctx));
    let mut output = test_ctx.run_rwasm_with_evm_input(runtime_ctx, &import_linker);
    let contract_address = Address::from_slice(output.data().output());
}
