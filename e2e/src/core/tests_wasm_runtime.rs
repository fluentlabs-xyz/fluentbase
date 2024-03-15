use crate::{core::testing_utils::TestingContext, test_helpers::wasm2rwasm};
use fluentbase_codec::Encoder;
use fluentbase_core::{
    account::Account,
    helpers::{calc_create2_address, calc_create_address, rwasm_exec},
};
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{
        WasmCallMethodInput,
        WasmCreate2MethodInput,
        WasmCreateMethodInput,
        WASM_CALL_METHOD_ID,
        WASM_CREATE2_METHOD_ID,
        WASM_CREATE_METHOD_ID,
    },
};
use fluentbase_runtime::{
    types::{address, Address, Bytes, B256, U256},
    Runtime,
    RuntimeContext,
};
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;
use hex_literal::hex;

#[test]
fn test_greeting_compilation() {
    let gas_limit: u32 = 10_000_000;
    let mut runtime_ctx = RuntimeContext::new(&[]);
    let _test_ctx = TestingContext::<(), false>::new(true, Some(&mut runtime_ctx));

    let greeting_deploy_wasm = include_bytes!("../../../examples/bin/greeting-deploy.wasm");
    let greeting_deploy_rwasm =
        fluentbase_core::helpers::wasm2rwasm(greeting_deploy_wasm.as_ref(), true);
    let contract_input = vec![];
    rwasm_exec(
        greeting_deploy_rwasm.as_ref(),
        &contract_input,
        gas_limit,
        true,
    );
    let mut out_len = LowLevelSDK::sys_output_size();
    let mut source_bytecode_out = vec![0u8; out_len as usize];
    LowLevelSDK::sys_read_output(source_bytecode_out.as_mut_ptr(), 0, out_len);
    assert_eq!(178, source_bytecode_out.len());
    println!("source_bytecode_out {:?}", &source_bytecode_out);
}

#[test]
fn test_wasm_create() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_account = Account {
        address: caller_address,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let expected_contract_address = calc_create_address(&caller_address, caller_account.nonce);
    let block_hash = B256::left_padding_from(&hex!("0123456789abcdef"));
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");

    let wasm_bytecode = include_bytes!("../../../examples/bin/greeting-deploy.wasm");

    let create_value = B256::left_padding_from(&hex!("1000"));
    let gas_limit: u32 = 10_000_000;
    let method_input =
        WasmCreateMethodInput::new(create_value.0, wasm_bytecode.to_vec(), gas_limit);
    let core_input = CoreInput::new(WASM_CREATE_METHOD_ID, method_input.encode_to_vec(0));
    let core_input_vec = core_input.encode_to_vec(0);

    const IS_RUNTIME: bool = true;
    let contract_wasm_binary = include_bytes!("../../../crates/core/bin/wcl_contract.wasm");
    let contract_rwasm_binary = wasm2rwasm(contract_wasm_binary.as_slice(), false);
    let mut runtime_ctx = RuntimeContext::new(contract_rwasm_binary);
    let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(true, Some(&mut runtime_ctx));
    let jzkt = runtime_ctx.jzkt().unwrap();
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
        .set_contract_input(Bytes::copy_from_slice(&core_input_vec))
        .set_contract_input_size(core_input_vec.len() as u32)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(wasm_bytecode))
        .set_contract_code_size(wasm_bytecode.len() as u32)
        .set_block_hash(block_hash)
        .set_block_coinbase(block_coinbase)
        .set_tx_caller(caller_address);
    test_ctx.apply_ctx(Some(&mut runtime_ctx));

    let import_linker = Runtime::<()>::new_sovereign_linker();
    let mut output = test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false, gas_limit);
    assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
    let output_vec = output.data().output();
    println!("output_vec {:?}", output_vec);
    assert!(output_vec.len() > 0);
    let contract_address = Address::from_slice(output_vec);
    println!("deployed contract_address {:x?}", contract_address);
    assert_eq!(expected_contract_address, contract_address);

    {
        let mut runtime_ctx = RuntimeContext::new(&[]);
        runtime_ctx.with_jzkt(jzkt.clone());
        let mut test_ctx =
            TestingContext::<(), { !IS_RUNTIME }>::new(false, Some(&mut runtime_ctx));
        test_ctx.apply_ctx(Some(&mut runtime_ctx));
        let account = Account::new_from_jzkt(&contract_address);
        assert_eq!(178, account.load_source_bytecode().len());
        assert_eq!(432, account.load_bytecode().len());
    }
}

#[test]
fn test_wasm_create2() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_account = Account {
        address: caller_address,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    let wasm_bytecode = include_bytes!("../../../examples/bin/greeting-deploy.wasm");
    let mut wasm_bytecode_hash = B256::default();
    keccak_hash::keccak_256(wasm_bytecode.as_ref(), wasm_bytecode_hash.as_mut_slice());

    let create_value = B256::left_padding_from(&hex!("1000"));
    let salt = B256::left_padding_from(&hex!("3749269486238462"));
    let gas_limit: u32 = 10_000_000;
    let method_input =
        WasmCreate2MethodInput::new(create_value.0, salt.0, wasm_bytecode.to_vec(), gas_limit);
    let core_input = CoreInput::new(WASM_CREATE2_METHOD_ID, method_input.encode_to_vec(0));
    let core_input_vec = core_input.encode_to_vec(0);

    let expected_contract_address =
        calc_create2_address(&caller_address, &salt, &wasm_bytecode_hash);

    const IS_RUNTIME: bool = true;
    let contract_wasm_binary = include_bytes!("../../../crates/core/bin/wcl_contract.wasm");
    let contract_rwasm_binary = wasm2rwasm(contract_wasm_binary.as_slice(), false);
    let mut runtime_ctx = RuntimeContext::new(contract_rwasm_binary);
    let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(true, Some(&mut runtime_ctx));
    let jzkt = runtime_ctx.jzkt().unwrap();
    test_ctx
        .try_add_account(&caller_account)
        .contract_input_wrapper
        .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
        .set_contract_input(Bytes::copy_from_slice(&core_input_vec))
        .set_contract_input_size(core_input_vec.len() as u32)
        .set_contract_caller(caller_address)
        .set_contract_bytecode(Bytes::copy_from_slice(wasm_bytecode))
        .set_contract_code_size(wasm_bytecode.len() as u32)
        .set_tx_caller(caller_address);
    test_ctx.apply_ctx(Some(&mut runtime_ctx));

    let import_linker = Runtime::<()>::new_sovereign_linker();
    let mut output = test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false, gas_limit);
    assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
    let output_vec = output.data().output();
    println!("output_vec {:?}", output_vec);
    assert!(output_vec.len() > 0);
    let contract_address = Address::from_slice(output_vec);
    println!("deployed contract_address {:x?}", contract_address);
    assert_eq!(expected_contract_address, contract_address);

    {
        let mut runtime_ctx = RuntimeContext::new(&[]);
        runtime_ctx.with_jzkt(jzkt.clone());
        let mut test_ctx =
            TestingContext::<(), { !IS_RUNTIME }>::new(false, Some(&mut runtime_ctx));
        test_ctx.apply_ctx(Some(&mut runtime_ctx));
        let account = Account::new_from_jzkt(&contract_address);
        assert_eq!(178, account.load_source_bytecode().len());
        assert_eq!(432, account.load_bytecode().len());
    }
}

#[test]
fn test_wasm_call_after_create() {
    let caller_address = address!("000000000000000000000000000000000000000c");
    let caller_account = Account {
        address: caller_address,
        balance: U256::from_be_slice(1000000000u128.to_be_bytes().as_slice()),
        ..Default::default()
    };

    const IS_RUNTIME: bool = true;
    let wcl_contract_wasm = include_bytes!("../../../crates/core/bin/wcl_contract.wasm");
    let wcl_contract_rwasm = wasm2rwasm(wcl_contract_wasm.as_slice(), false);
    let block_coinbase: Address = address!("0000000000000000000000000000000000000012");
    let gas_limit: u32 = 10_000_000;
    let create_value = B256::left_padding_from(&hex!("1000"));
    let call_value = B256::left_padding_from(&hex!("00"));
    let deploy_wasm = include_bytes!("../../../examples/bin/greeting-deploy.wasm");
    let import_linker = Runtime::<()>::new_sovereign_linker();

    let (jzkt, deployed_contract_address) = {
        let expected_contract_address = calc_create_address(&caller_address, caller_account.nonce);
        let method_input =
            WasmCreateMethodInput::new(create_value.0, deploy_wasm.to_vec(), gas_limit);
        let core_input = CoreInput::new(WASM_CREATE_METHOD_ID, method_input.encode_to_vec(0));
        let core_input_vec = core_input.encode_to_vec(0);

        let mut runtime_ctx = RuntimeContext::new(wcl_contract_rwasm.clone());
        let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(true, Some(&mut runtime_ctx));
        let jzkt = runtime_ctx.jzkt().unwrap();
        test_ctx
            .try_add_account(&caller_account)
            .contract_input_wrapper
            .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
            .set_contract_bytecode(Bytes::copy_from_slice(deploy_wasm))
            .set_contract_caller(caller_address)
            .set_contract_input(Bytes::copy_from_slice(&core_input_vec))
            .set_contract_input_size(core_input_vec.len() as u32)
            .set_contract_code_size(deploy_wasm.len() as u32)
            .set_block_coinbase(block_coinbase);
        test_ctx.apply_ctx(Some(&mut runtime_ctx));

        let mut output =
            test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false, gas_limit);
        assert_eq!(ExitCode::Ok.into_i32(), output.data().exit_code());
        let output_vec = output.data().output();
        println!("output_vec {:?}", output_vec);
        assert!(output_vec.len() > 0);
        let contract_address = Address::from_slice(output_vec);
        println!("deployed contract_address {:x?}", contract_address);
        assert_eq!(expected_contract_address, contract_address);

        {
            let mut runtime_ctx = RuntimeContext::new(&[]);
            runtime_ctx.with_jzkt(jzkt.clone());
            let mut test_ctx =
                TestingContext::<(), { !IS_RUNTIME }>::new(false, Some(&mut runtime_ctx));
            test_ctx.apply_ctx(Some(&mut runtime_ctx));
            let account = Account::new_from_jzkt(&contract_address);
            assert_eq!(178, account.load_source_bytecode().len());
            assert_eq!(432, account.load_bytecode().len());
        }

        (jzkt, contract_address)
    };

    let (jzkt) = {
        let ecl_method_input = WasmCallMethodInput::new(
            deployed_contract_address.into_array(),
            call_value.0,
            vec![],
            gas_limit,
        );
        let ecl_core_input = CoreInput::new(WASM_CALL_METHOD_ID, ecl_method_input.encode_to_vec(0));
        let ecl_core_input_vec = ecl_core_input.encode_to_vec(0);

        let mut runtime_ctx = RuntimeContext::new(wcl_contract_rwasm.clone());
        runtime_ctx.with_jzkt(jzkt.clone());
        let mut test_ctx = TestingContext::<(), IS_RUNTIME>::new(false, Some(&mut runtime_ctx));
        test_ctx
            .try_add_account(&caller_account)
            .contract_input_wrapper
            .set_journal_checkpoint(runtime_ctx.jzkt().unwrap().borrow_mut().checkpoint().into())
            .set_contract_bytecode(Bytes::copy_from_slice(deploy_wasm))
            .set_contract_address(deployed_contract_address)
            .set_contract_input(Bytes::copy_from_slice(&ecl_core_input_vec))
            .set_contract_input_size(ecl_core_input_vec.len() as u32)
            .set_contract_caller(caller_address);
        test_ctx.apply_ctx(Some(&mut runtime_ctx));

        let mut output_res =
            test_ctx.run_rwasm_with_input(runtime_ctx, &import_linker, false, gas_limit);
        assert_eq!(ExitCode::Ok.into_i32(), output_res.data().exit_code());
        let output = output_res.data().output();
        println!("output_vec {:?}", output);
        assert!(output.len() > 0);
        assert_eq!(
            &[72, 101, 108, 108, 111, 44, 32, 87, 111, 114, 108, 100],
            output.as_slice(),
        );

        (jzkt)
    };
}
