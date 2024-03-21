use crate::{
    assets::evm_test_contract::{
        EVM_CONTRACT_BYTECODE1,
        EVM_CONTRACT_BYTECODE1_METHOD_SAY_HELLO_WORLD_STR_ID,
    },
    core::utils::TestingContext,
};
use fluentbase_codec::Encoder;
use fluentbase_core::{
    consts::ECL_CONTRACT_ADDRESS,
    helpers::{calc_create_address, wasm2rwasm},
    Account,
};
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{
        EvmCallMethodInput,
        EvmCreateMethodInput,
        EVM_CALL_METHOD_ID,
        EVM_CREATE_METHOD_ID,
    },
};
use fluentbase_runtime::{Runtime, RuntimeContext};
use fluentbase_types::{address, Address, Bytes, ExitCode, B256, STATE_DEPLOY, STATE_MAIN, U256};
use hex_literal::hex;

#[test]
fn test_evm_create() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_account = Account {
        address: caller_address,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let expected_contract_address = calc_create_address(&caller_address, caller_account.nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_code = EVM_CONTRACT_BYTECODE1;

    let value = B256::left_padding_from(&hex!("1000"));
    let gas_limit: u32 = 10_000_000;
    let evm_create_method_input =
        EvmCreateMethodInput::new(value.0, contract_input_code.to_vec(), gas_limit);
    let evm_create_core_input = CoreInput::new(
        EVM_CREATE_METHOD_ID,
        evm_create_method_input.encode_to_vec(0),
    );
    let evm_create_core_input_vec = evm_create_core_input.encode_to_vec(0);

    const IS_RUNTIME: bool = true;
    let evm_contract_wasm_binary =
        include_bytes!("../../../crates/contracts/assets/ecl_contract.wasm");
    let evm_contract_rwasm_binary = wasm2rwasm(evm_contract_wasm_binary.as_slice()).unwrap();
    let mut runtime_ctx = RuntimeContext::new(evm_contract_rwasm_binary);
    runtime_ctx.with_state(STATE_MAIN);
    let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(true, Some(&mut runtime_ctx));
    test_ctx.try_add_account(&caller_account);
    test_ctx
        .contract_input_wrapper
        .set_journal_checkpoint(
            runtime_ctx
                .jzkt()
                .unwrap()
                .borrow_mut()
                .checkpoint()
                .to_u64(),
        )
        .set_contract_input(Bytes::copy_from_slice(&evm_create_core_input_vec))
        .set_contract_input_size(evm_create_core_input_vec.len() as u32)
        .set_env_chain_id(env_chain_id)
        .set_contract_caller(caller_address)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address);
    test_ctx.apply_ctx(Some(&mut runtime_ctx));

    let import_linker = Runtime::<()>::new_sovereign_linker();
    let output = test_ctx.run_rwasm_with_input(runtime_ctx, import_linker, false, gas_limit);
    assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
    let contract_address_vec = output.data().output();
    let contract_address = Address::from_slice(contract_address_vec);

    assert_eq!(expected_contract_address, contract_address);
}

#[test]
fn test_evm_call_after_create() {
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
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let env_chain_id = 1;

    let contract_input_code = EVM_CONTRACT_BYTECODE1;
    let gas_limit: u32 = 10_000_000;
    const IS_RUNTIME: bool = true;
    let import_linker = Runtime::<()>::new_sovereign_linker();
    let ecl_wasm = include_bytes!("../../../crates/contracts/assets/ecl_contract.wasm");
    let ecl_rwasm = wasm2rwasm(ecl_wasm.as_slice()).unwrap();
    let create_value = B256::left_padding_from(&hex!("1000"));
    let call_value = B256::left_padding_from(&hex!("00"));

    let (jzkt, deployed_contract_address) = {
        let evm_create_method_input =
            EvmCreateMethodInput::new(create_value.0, contract_input_code.to_vec(), gas_limit);
        let evm_create_core_input = CoreInput::new(
            EVM_CREATE_METHOD_ID,
            evm_create_method_input.encode_to_vec(0),
        );
        let evm_create_core_input_vec = evm_create_core_input.encode_to_vec(0);

        let mut runtime_ctx = RuntimeContext::new(ecl_rwasm.clone());
        runtime_ctx.with_state(STATE_DEPLOY);
        let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(true, Some(&mut runtime_ctx));
        test_ctx
            .try_add_account(&caller_account)
            .contract_input_wrapper
            .set_journal_checkpoint(
                runtime_ctx
                    .jzkt()
                    .unwrap()
                    .borrow_mut()
                    .checkpoint()
                    .to_u64(),
            )
            .set_contract_input(Bytes::copy_from_slice(&evm_create_core_input_vec))
            .set_contract_input_size(evm_create_core_input_vec.len() as u32)
            .set_env_chain_id(env_chain_id)
            .set_contract_caller(caller_address)
            .set_block_hash(block_hash)
            .set_block_coinbase(block_coinbase)
            .set_tx_caller(caller_address);
        test_ctx.apply_ctx(Some(&mut runtime_ctx));
        let jzkt = runtime_ctx.jzkt().clone();
        let output =
            test_ctx.run_rwasm_with_input(runtime_ctx, import_linker.clone(), false, gas_limit);
        assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
        let output = output.data().output();
        assert!(output.len() > 0);
        let contract_address = Address::from_slice(output);
        assert_eq!(&expected_contract_address, &contract_address);

        (jzkt, contract_address)
    };

    {
        let evm_call_method_input = EvmCallMethodInput::new(
            deployed_contract_address.into_array(),
            call_value.0,
            EVM_CONTRACT_BYTECODE1_METHOD_SAY_HELLO_WORLD_STR_ID.to_vec(),
            gas_limit,
        );
        let evm_call_core_input =
            CoreInput::new(EVM_CALL_METHOD_ID, evm_call_method_input.encode_to_vec(0));
        let evm_call_core_input_vec = evm_call_core_input.encode_to_vec(0);

        let mut runtime_ctx = RuntimeContext::new(ecl_rwasm.clone());
        runtime_ctx.with_jzkt(jzkt.unwrap());
        let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(false, Some(&mut runtime_ctx));
        test_ctx
            .contract_input_wrapper
            .set_journal_checkpoint(
                runtime_ctx
                    .jzkt()
                    .unwrap()
                    .borrow_mut()
                    .checkpoint()
                    .to_u64(),
            )
            .set_contract_input(Bytes::copy_from_slice(&evm_call_core_input_vec))
            .set_contract_input_size(evm_call_core_input_vec.len() as u32)
            .set_contract_address(deployed_contract_address);
        test_ctx.apply_ctx(Some(&mut runtime_ctx));
        let output_res =
            test_ctx.run_rwasm_with_input(runtime_ctx, import_linker, false, gas_limit);
        assert_eq!(ExitCode::Ok.into_i32(), output_res.data().exit_code());
        let output = output_res.data().output();
        assert_eq!(
            &[
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 11, 72, 101, 108, 108, 111, 32, 87, 111, 114, 108, 100, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ],
            output.as_slice(),
        );
    };
}

#[test]
fn test_evm_call_from_wasm() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_account = Account {
        address: caller_address,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };
    let gas_limit: u32 = 10_000_000;

    const IS_RUNTIME: bool = true;
    let import_linker = Runtime::<()>::new_sovereign_linker();

    let jzkt = {
        let mut runtime_ctx = RuntimeContext::new(&[]);
        let mut test_ctx =
            TestingContext::<(), { !IS_RUNTIME }>::new(false, Some(&mut runtime_ctx));
        let jzkt = test_ctx.init_jzkt(Some(&mut runtime_ctx));
        let mut ecl_account = Account::new_from_jzkt(&ECL_CONTRACT_ADDRESS);
        ecl_account.update_source_bytecode(
            &include_bytes!("../../../crates/contracts/assets/ecl_contract.wasm").into(),
        );
        ecl_account.update_rwasm_bytecode(
            &include_bytes!("../../../crates/contracts/assets/ecl_contract.rwasm").into(),
        );
        ecl_account.write_to_jzkt();
        println!(
            "ecl_account.rwasm_bytecode_hash {}",
            ecl_account.rwasm_bytecode_hash
        );
        Account::commit();
        jzkt
    };

    let (jzkt, deployed_contract_address) = {
        let expected_contract_address = calc_create_address(&caller_address, caller_account.nonce);
        let contract_input_code = EVM_CONTRACT_BYTECODE1;
        let create_value = B256::left_padding_from(&hex!("1000"));
        let evm_create_method_input =
            EvmCreateMethodInput::new(create_value.0, contract_input_code.to_vec(), gas_limit);
        let evm_create_core_input = CoreInput::new(
            EVM_CREATE_METHOD_ID,
            evm_create_method_input.encode_to_vec(0),
        );
        let evm_create_core_input_vec = evm_create_core_input.encode_to_vec(0);
        let wasm_binary = include_bytes!("../../../crates/contracts/assets/ecl_contract.wasm");
        let rwasm_binary = wasm2rwasm(wasm_binary).unwrap();
        let mut runtime_ctx = RuntimeContext::new(rwasm_binary.clone());
        runtime_ctx.with_state(STATE_MAIN);
        runtime_ctx.with_jzkt(jzkt);
        let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(false, Some(&mut runtime_ctx));
        test_ctx
            .try_add_account(&caller_account)
            .contract_input_wrapper
            .set_journal_checkpoint(
                runtime_ctx
                    .jzkt()
                    .unwrap()
                    .borrow_mut()
                    .checkpoint()
                    .to_u64(),
            )
            .set_contract_gas_limit(gas_limit.into())
            .set_contract_input(Bytes::copy_from_slice(&evm_create_core_input_vec))
            .set_contract_input_size(evm_create_core_input_vec.len() as u32)
            .set_contract_caller(caller_address);
        test_ctx.apply_ctx(Some(&mut runtime_ctx));
        let jzkt = runtime_ctx.jzkt().clone();
        let output =
            test_ctx.run_rwasm_with_input(runtime_ctx, import_linker.clone(), false, gas_limit);
        assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
        let output = output.data().output();
        assert!(output.len() > 0);
        let evm_contract_address = Address::from_slice(output);
        assert_eq!(&expected_contract_address, &evm_contract_address);

        (jzkt, evm_contract_address)
    };

    {
        let evm_call_from_wasm_wasm_binary =
            include_bytes!("../../../examples/bin/evm_call_from_wasm.wasm");
        let evm_call_from_wasm_rwasm_binary = wasm2rwasm(evm_call_from_wasm_wasm_binary).unwrap();

        let mut runtime_ctx = RuntimeContext::new(evm_call_from_wasm_rwasm_binary);
        runtime_ctx.with_state(STATE_MAIN);
        runtime_ctx.with_jzkt(jzkt.unwrap());
        let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(false, Some(&mut runtime_ctx));
        let contract_input = EVM_CONTRACT_BYTECODE1_METHOD_SAY_HELLO_WORLD_STR_ID;
        test_ctx
            .contract_input_wrapper
            .set_journal_checkpoint(
                runtime_ctx
                    .jzkt()
                    .unwrap()
                    .borrow_mut()
                    .checkpoint()
                    .to_u64(),
            )
            .set_contract_gas_limit(gas_limit.into())
            .set_contract_input_size(contract_input.len() as u32)
            .set_contract_input(contract_input.into())
            .set_contract_address(deployed_contract_address)
            .set_contract_caller(caller_address);
        test_ctx.apply_ctx(Some(&mut runtime_ctx));
        let output = test_ctx.run_rwasm_with_input(runtime_ctx, import_linker, false, gas_limit);
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
