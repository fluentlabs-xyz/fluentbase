use alloc::vec::Vec;
use core::str::from_utf8;
use fluentbase_erc20::{
    common::{fixed_bytes_from_u256, sig_to_bytes, u256_from_bytes_slice_try},
    consts::{
        ERR_ALREADY_PAUSED,
        ERR_ALREADY_UNPAUSED,
        SIG_ALLOWANCE,
        SIG_APPROVE,
        SIG_BALANCE_OF,
        SIG_DECIMALS,
        SIG_MINT,
        SIG_NAME,
        SIG_PAUSE,
        SIG_SYMBOL,
        SIG_TOTAL_SUPPLY,
        SIG_TRANSFER,
        SIG_TRANSFER_FROM,
        SIG_UNPAUSE,
    },
    storage::{Feature, InitialSettings, DECIMALS_DEFAULT},
};
use fluentbase_sdk::{address, Address, Bytes, U256};
use fluentbase_sdk_testing::EvmTestingContext;
use fluentbase_types::{ContractContextV1, PRECOMPILE_ERC20};
use revm::context::result::ExecutionResult;
use std::ops::Add;

#[test]
fn erc20_test() {
    let mut ctx = EvmTestingContext::default();
    const DEPLOYER_ADDR: Address = address!("1111111111111111111111111111111111111111");
    const USER_ADDR: Address = address!("2222222222222222222222222222222222222222");
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_ERC20,
        ..Default::default()
    });
    let mut initial_settings = InitialSettings::new();
    let total_supply = U256::from(0xffff_ffffu64);
    let token_name = "NaMe";
    let token_symbol = "SyMbOl";
    let decimals = U256::from(DECIMALS_DEFAULT);
    let deployer_1_2_transfer = 12345678;
    let deployer_2_1_allowance = 1234567;
    let deployer_2_1_transfer_from = 1234;
    let amount_to_mint = 93842;
    initial_settings.add_feature(Feature::InitialSupply {
        amount: fixed_bytes_from_u256(&total_supply),
        owner: DEPLOYER_ADDR.into(),
        decimals: DECIMALS_DEFAULT,
    });
    initial_settings.add_feature(Feature::Meta {
        name: token_name.as_bytes().to_vec(),
        symbol: token_symbol.as_bytes().to_vec(),
    });
    initial_settings.add_feature(Feature::Pausable {
        pauser: DEPLOYER_ADDR.into(),
    });
    initial_settings.add_feature(Feature::Mintable {
        minter: DEPLOYER_ADDR.into(),
    });

    let init_bytecode = initial_settings
        .try_encode_for_deploy()
        .expect("failed to encode settings for deploy")
        .into();
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDR, init_bytecode);

    fn call_with_sig(
        ctx: &mut EvmTestingContext,
        input: Bytes,
        caller: &Address,
        callee: &Address,
    ) -> Vec<u8> {
        let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
        println!("result: {:?}", result);
        assert!(result.is_success());
        let output_data = result.output().unwrap().to_vec();
        output_data
    }

    fn call_with_sig_revert(
        ctx: &mut EvmTestingContext,
        input: Bytes,
        caller: &Address,
        callee: &Address,
    ) -> u32 {
        let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
        match &result {
            ExecutionResult::Revert {
                gas_used: _,
                output,
            } => {
                let error_code = u32::from_be_bytes(output[32..].try_into().unwrap());
                error_code
            }
            _ => {
                panic!("expected revert, got: {:?}", &result)
            }
        }
    }

    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_TOTAL_SUPPLY));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let total_supply_recovered = U256::from_be_slice(output_data.as_ref());
    assert_eq!(total_supply, total_supply_recovered);

    // SIG_TRANSFER
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_TRANSFER));
    input.extend(USER_ADDR);
    input.extend(&fixed_bytes_from_u256(&U256::from(deployer_1_2_transfer)));
    println!("SIG_TRANSFER input hex: {}", hex::encode(&input));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let expected = U256::from(1);
    let recovered = u256_from_bytes_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    // SIG_BALANCE_OF
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_BALANCE_OF));
    input.extend(USER_ADDR.as_slice());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let expected = U256::from(deployer_1_2_transfer);
    let recovered = u256_from_bytes_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    // SIG_NAME
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_NAME));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered = from_utf8(output_data.as_ref()).expect("output_data should be utf8");
    assert_eq!(token_name, recovered);

    // SIG_SYMBOL
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_SYMBOL));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered = from_utf8(output_data.as_ref()).expect("output_data should be utf8");
    assert_eq!(token_symbol, recovered);

    // SIG_DECIMALS
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_DECIMALS));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered =
        u256_from_bytes_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(decimals, recovered);

    // SIG_TOTAL_SUPPLY
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_TOTAL_SUPPLY));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered =
        u256_from_bytes_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(total_supply, recovered);

    // ALLOWANCE TESTS:
    // SIG_ALLOWANCE: before approve
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_ALLOWANCE));
    input.extend(USER_ADDR);
    input.extend(DEPLOYER_ADDR);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered =
        u256_from_bytes_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(U256::from(0), recovered);
    // SIG_APPROVE
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_APPROVE));
    input.extend(USER_ADDR);
    input.extend(DEPLOYER_ADDR);
    input.extend(&fixed_bytes_from_u256(&U256::from(deployer_2_1_allowance)));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered =
        u256_from_bytes_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(U256::from(1), recovered);
    // SIG_ALLOWANCE: after approve
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_ALLOWANCE));
    input.extend(USER_ADDR);
    input.extend(DEPLOYER_ADDR);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered =
        u256_from_bytes_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(U256::from(deployer_2_1_allowance), recovered);
    // SIG_TRANSFER_FROM
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_TRANSFER_FROM));
    input.extend(USER_ADDR);
    input.extend(DEPLOYER_ADDR);
    input.extend(fixed_bytes_from_u256(&U256::from(
        deployer_2_1_transfer_from,
    )));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered =
        u256_from_bytes_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(U256::from(1), recovered);
    // SIG_ALLOWANCE: after transfer from
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_ALLOWANCE));
    input.extend(USER_ADDR);
    input.extend(DEPLOYER_ADDR);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered =
        u256_from_bytes_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(
        U256::from(deployer_2_1_allowance - deployer_2_1_transfer_from),
        recovered
    );
    // SIG_BALANCE_OF: after transfer from
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_BALANCE_OF));
    input.extend(USER_ADDR.as_slice());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let expected = U256::from(deployer_1_2_transfer - deployer_2_1_transfer_from);
    let recovered = u256_from_bytes_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    // SIG_PAUSE
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_PAUSE));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let expected = U256::from(1);
    let recovered = u256_from_bytes_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);
    // SIG_PAUSE: 2nd time
    // println!(
    //     "ERR_ALREADY_PAUSED bytes: {:x?}",
    //     sig_to_bytes(ERR_ALREADY_PAUSED)
    // );
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_PAUSE));
    let error_code = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(error_code, ERR_ALREADY_PAUSED);
    // SIG_UNPAUSE
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_UNPAUSE));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let expected = U256::from(1);
    let recovered = u256_from_bytes_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);
    // SIG_UNPAUSE: 2nd time
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_UNPAUSE));
    let error_code = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(error_code, ERR_ALREADY_UNPAUSED);

    // SIG_MINT
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_MINT));
    input.extend(USER_ADDR.as_slice());
    input.extend(fixed_bytes_from_u256(&U256::from(amount_to_mint)));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let expected = U256::from(1);
    let recovered = u256_from_bytes_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);
    // SIG_BALANCE_OF: after mint
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_BALANCE_OF));
    input.extend(USER_ADDR.as_slice());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let expected = U256::from(deployer_1_2_transfer - deployer_2_1_transfer_from + amount_to_mint);
    let recovered = u256_from_bytes_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);
    // SIG_TOTAL_SUPPLY: after mint
    let mut input = Vec::<u8>::new();
    input.extend(sig_to_bytes(SIG_TOTAL_SUPPLY));
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    println!("output_data: {:?}", output_data);
    let recovered =
        u256_from_bytes_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(total_supply.add(U256::from(amount_to_mint)), recovered);
}
