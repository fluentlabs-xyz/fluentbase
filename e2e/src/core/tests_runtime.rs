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
use fluentbase_types::ExitCode;
use hex_literal::hex;
use keccak_hash::keccak;

#[test]
fn test_create() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_account = Account {
        address: caller_address,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let expected_contract_address = calc_create_address(&caller_address, caller_account.nonce);
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
    let evm_contract_wasm_binary = include_bytes!("../../../crates/core/bin/evm_contract.wasm");
    let evm_contract_rwasm_binary = wasm2rwasm(evm_contract_wasm_binary.as_slice(), false);
    let mut runtime_ctx = RuntimeContext::new(evm_contract_rwasm_binary);
    let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(true, Some(&mut runtime_ctx));
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
        .set_contract_input(Bytes::copy_from_slice(&evm_create_core_input_vec))
        .set_contract_input_size(evm_create_core_input_vec.len() as u32)
        .set_env_chain_id(env_chain_id)
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
    let mut output = test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false);
    let contract_address_vec = output.data().output();
    let contract_address = Address::from_slice(contract_address_vec);

    assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
    println!("contract_address {:x?}", contract_address);
    assert_eq!(expected_contract_address, contract_address);
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
    let call_value = B256::left_padding_from(&hex!("00"));
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
    let wasm_binary = include_bytes!("../../../crates/core/bin/evm_contract.wasm");
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
    let jzkt = runtime_ctx.jzkt().clone();
    let mut output = test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false);
    assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
    let contract_address = Address::from_slice(output.data().output());
    println!("contract_address {:x?}", contract_address);
    assert_eq!(&expected_contract_address, &contract_address);

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
    runtime_ctx.with_jzkt(jzkt.unwrap());
    test_ctx
        .contract_input_wrapper
        .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
        .reset_contract_input()
        .set_contract_input(Bytes::copy_from_slice(&evm_call_core_input_vec))
        .reset_contract_input_size()
        .set_contract_input_size(evm_call_core_input_vec.len() as u32)
        .set_contract_address(contract_address)
        .set_contract_is_static(contract_is_static);
    test_ctx.apply_ctx(Some(&mut runtime_ctx));
    let mut output = test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false);
    assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
    let call_output = output.data().output();
    assert_eq!(
        &[
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 11, 72, 101, 108, 108, 111, 32, 87, 111, 114, 108, 100, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ],
        call_output.as_slice(),
    );
}

#[test]
fn test_call_evm_from_wasm() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    println!("caller_address {:x?}", caller_address);
    let caller_account = Account {
        address: caller_address,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };
    let gas_limit: u32 = 10_000_000;

    const IS_RUNTIME: bool = true;
    const ECL_CONTRACT_ADDRESS: Address = address!("0000000000000000000000000000000000000001");
    let import_linker = Runtime::<()>::new_sovereign_linker();

    let (jzkt) = {
        let mut runtime_ctx = RuntimeContext::new(&[]);
        let mut test_ctx =
            TestingContext::<(), { !IS_RUNTIME }>::new(false, Some(&mut runtime_ctx));
        let mut jzkt = test_ctx.init_jzkt(Some(&mut runtime_ctx));
        let mut ecl_account = Account::new_from_jzkt(&ECL_CONTRACT_ADDRESS);
        ecl_account.update_source_bytecode(
            &include_bytes!("../../../crates/core/bin/evm_contract.wasm").into(),
        );
        ecl_account
            .update_bytecode(&include_bytes!("../../../crates/core/bin/evm_contract.rwasm").into());
        ecl_account.write_to_jzkt();
        Account::commit();

        (jzkt)
    };

    let (jzkt, evm_test_contract_address) = {
        let expected_contract_address = calc_create_address(&caller_address, caller_account.nonce);
        let evm_contract_input_bytes = CONTRACT_BYTECODE1;
        let create_value = B256::left_padding_from(&hex!("1000"));
        let evm_create_method_input =
            EvmCreateMethodInput::new(create_value.0, evm_contract_input_bytes.to_vec(), gas_limit);
        let evm_create_core_input = CoreInput::new(
            EVM_CREATE_METHOD_ID,
            evm_create_method_input.encode_to_vec(0),
        );
        let evm_create_core_input_vec = evm_create_core_input.encode_to_vec(0);
        let wasm_binary = include_bytes!("../../../crates/core/bin/evm_contract.wasm");
        let rwasm_binary = wasm2rwasm(wasm_binary.as_slice(), false);
        let mut runtime_ctx = RuntimeContext::new(rwasm_binary.clone());
        runtime_ctx.with_jzkt(jzkt);
        let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(false, Some(&mut runtime_ctx));
        test_ctx
            .try_add_account(&caller_account)
            .contract_input_wrapper
            .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
            .set_contract_gas_limit(gas_limit.into())
            .set_contract_input(Bytes::copy_from_slice(&evm_create_core_input_vec))
            .set_contract_input_size(evm_create_core_input_vec.len() as u32)
            .set_contract_caller(caller_address)
            .set_contract_bytecode(Bytes::copy_from_slice(CONTRACT_BYTECODE1))
            .set_contract_code_size(CONTRACT_BYTECODE1.len() as u32)
            .set_contract_code_hash(B256::from_slice(keccak(CONTRACT_BYTECODE1).as_bytes()))
            .set_tx_caller(caller_address);
        test_ctx.apply_ctx(Some(&mut runtime_ctx));
        let jzkt = runtime_ctx.jzkt().clone();
        let mut output = test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false);
        assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
        let output = output.data().output();
        let evm_contract_address = Address::from_slice(output);
        println!("evm_contract_address {:x?}", evm_contract_address);
        assert_eq!(&expected_contract_address, &evm_contract_address);

        (jzkt, evm_contract_address)
    };

    {
        let evm_call_from_wasm_wasm_binary =
            include_bytes!("../../../examples/bin/evm_call_from_wasm.wasm");
        let evm_call_from_wasm_rwasm_binary =
            wasm2rwasm(evm_call_from_wasm_wasm_binary.as_slice(), false);

        let mut runtime_ctx = RuntimeContext::new(evm_call_from_wasm_rwasm_binary);
        runtime_ctx.with_jzkt(jzkt.unwrap());
        let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(false, Some(&mut runtime_ctx));
        test_ctx
            .contract_input_wrapper
            .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
            .set_contract_gas_limit(gas_limit.into())
            .set_contract_input_size(CONTRACT_BYTECODE1_METHOD_SAY_HELLO_WORLD_STR_ID.len() as u32)
            .set_contract_input(CONTRACT_BYTECODE1_METHOD_SAY_HELLO_WORLD_STR_ID.into())
            .set_contract_address(evm_test_contract_address)
            .set_contract_caller(caller_address);
        test_ctx.apply_ctx(Some(&mut runtime_ctx));
        let mut output = test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false);
        assert_eq!(output.data().exit_code(), ExitCode::Ok.into_i32());
        let call_output = output.data().output();
        assert_eq!(
            &[
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 11, 72, 101, 108, 108, 111, 32, 87, 111, 114, 108, 100, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ],
            call_output.as_slice(),
        );
    }
}
