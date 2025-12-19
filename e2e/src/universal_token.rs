use crate::EvmTestingContextWithGenesis;
use alloc::vec::Vec;
use core::str::from_utf8;
use fluentbase_sdk::{
    bincode_helpers::decode, crypto::crypto_keccak256, derive::derive_keccak256,
    system::RuntimeExecutionOutcomeV1, Address, Bytes, ContractContextV1,
    PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME, U256,
};
use fluentbase_testing::EvmTestingContext;
use fluentbase_universal_token::{
    command::{
        AllowanceCommand, ApproveCommand, BalanceOfCommand, MintCommand, TransferCommand,
        TransferFromCommand, UniversalTokenCommand,
    },
    consts::{
        ERR_ALREADY_PAUSED, ERR_ALREADY_UNPAUSED, ERR_CONTRACT_NOT_MINTABLE,
        ERR_CONTRACT_NOT_PAUSABLE, ERR_INSUFFICIENT_ALLOWANCE, SIG_DECIMALS, SIG_NAME, SIG_PAUSE,
        SIG_SYMBOL, SIG_TOTAL_SUPPLY, SIG_UNPAUSE,
    },
    storage::{InitialSettings, DECIMALS_DEFAULT},
};
use revm::context::result::ExecutionResult;
use std::ops::Add;

const DEPLOYER_ADDR: Address = Address::repeat_byte(1);
const USER_ADDR: Address = Address::repeat_byte(2);
const RECIPIENT_ADDR: Address = Address::repeat_byte(3);

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
) -> Bytes {
    let result = ctx.call_evm_tx(*caller, *callee, input, None, None);
    match result {
        ExecutionResult::Revert {
            gas_used: _,
            output,
        } => output,
        _ => {
            panic!("expected revert, got: {:?}", &result)
        }
    }
}

pub fn u256_from_slice_try(value: &[u8]) -> Option<U256> {
    U256::try_from_be_slice(value)
}

#[test]
fn no_plugins_enabled_test() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    let mut initial_settings = InitialSettings {
        token_name: Default::default(),
        token_symbol: Default::default(),
        decimals: DECIMALS_DEFAULT,
        initial_supply: U256::from(0xffff_ffffu64),
        minter: None,
        pauser: None,
    };
    let total_supply = U256::from(0xffff_ffffu64);
    let amount_to_mint = 93842;

    let init_bytecode: Bytes = initial_settings.encode_with_prefix();
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDR, init_bytecode);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_TOTAL_SUPPLY.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let total_supply_recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(total_supply, total_supply_recovered);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_PAUSE.to_be_bytes());
    let output = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_CONTRACT_NOT_PAUSABLE, evm_exit_code);

    let mut input = Vec::<u8>::new();
    MintCommand {
        to: USER_ADDR,
        amount: U256::from(amount_to_mint),
    }
    .encode_for_send(&mut input);
    let output = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_CONTRACT_NOT_MINTABLE, evm_exit_code);
}

#[test]
fn mixed_test() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    let mut initial_settings = InitialSettings {
        token_name: "NaMe".into(),
        token_symbol: "SyMbOl".into(),
        decimals: DECIMALS_DEFAULT,
        initial_supply: U256::from(0xffff_ffffu64),
        minter: Some(DEPLOYER_ADDR),
        pauser: Some(DEPLOYER_ADDR),
    };
    let total_supply = U256::from(0xffff_ffffu64);
    let token_name = "NaMe";
    let token_symbol = "SyMbOl";
    let decimals = U256::from(DECIMALS_DEFAULT);
    let deployer_1_2_transfer = 12345678;
    let deployer_2_1_allowance = 1234567;
    let deployer_2_1_transfer_from = 1234;
    let amount_to_mint = 93842;

    let init_bytecode = initial_settings.encode_with_prefix();
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDR, init_bytecode);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_TOTAL_SUPPLY.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let total_supply_recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(total_supply, total_supply_recovered);

    let mut input = Vec::<u8>::new();
    TransferCommand {
        to: USER_ADDR,
        amount: U256::from(deployer_1_2_transfer),
    }
    .encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected = U256::from(1);
    let recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    let mut input = Vec::<u8>::new();
    BalanceOfCommand { owner: USER_ADDR }.encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected = U256::from(deployer_1_2_transfer);
    let recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_NAME.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = from_utf8(output_data.as_ref()).expect("output_data should be utf8");
    assert_eq!(token_name, recovered);

    // SIG_SYMBOL
    let mut input = Vec::<u8>::new();
    input.extend(SIG_SYMBOL.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = from_utf8(output_data.as_ref()).expect("output_data should be utf8");
    assert_eq!(token_symbol, recovered);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_DECIMALS.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(decimals, recovered);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_TOTAL_SUPPLY.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(total_supply, recovered);

    // before approve
    let mut input = Vec::<u8>::new();
    TransferFromCommand {
        from: USER_ADDR,
        to: DEPLOYER_ADDR,
        amount: U256::from(deployer_2_1_transfer_from),
    }
    .encode_for_send(&mut input);
    let output = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &USER_ADDR,
        &contract_address,
    );
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_INSUFFICIENT_ALLOWANCE, evm_exit_code);

    // before approve
    let mut input = Vec::<u8>::new();
    AllowanceCommand {
        owner: USER_ADDR,
        spender: DEPLOYER_ADDR,
    }
    .encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(U256::from(0), recovered);

    let mut input = Vec::<u8>::new();
    ApproveCommand {
        spender: DEPLOYER_ADDR,
        amount: U256::from(deployer_2_1_allowance),
    }
    .encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &USER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(U256::from(1), recovered);

    // after approve
    let mut input = Vec::<u8>::new();
    AllowanceCommand {
        owner: USER_ADDR,
        spender: DEPLOYER_ADDR,
    }
    .encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is a u256 repr");
    assert_eq!(U256::from(deployer_2_1_allowance), recovered);

    let mut input = Vec::<u8>::new();
    TransferFromCommand {
        from: USER_ADDR,
        to: DEPLOYER_ADDR,
        amount: U256::from(deployer_2_1_transfer_from),
    }
    .encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(U256::from(1), recovered);

    // after transfer from
    let mut input = Vec::<u8>::new();
    AllowanceCommand {
        owner: USER_ADDR,
        spender: DEPLOYER_ADDR,
    }
    .encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(
        U256::from(deployer_2_1_allowance - deployer_2_1_transfer_from),
        recovered
    );

    // after transfer from
    let mut input = Vec::<u8>::new();
    BalanceOfCommand { owner: USER_ADDR }.encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected = U256::from(deployer_1_2_transfer - deployer_2_1_transfer_from);
    let recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_PAUSE.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected = U256::from(1);
    let recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    // 2nd time
    let mut input = Vec::<u8>::new();
    input.extend(SIG_PAUSE.to_be_bytes());
    let output = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_ALREADY_PAUSED, evm_exit_code);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_UNPAUSE.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected = U256::from(1);
    let recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    // 2nd time
    let mut input = Vec::<u8>::new();
    input.extend(SIG_UNPAUSE.to_be_bytes());
    let output = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_ALREADY_UNPAUSED, evm_exit_code);

    // SIG_MINT
    let mut input = Vec::<u8>::new();
    MintCommand {
        to: USER_ADDR,
        amount: U256::from(amount_to_mint),
    }
    .encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected = U256::from(1);
    let recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    // after mint
    let mut input = Vec::<u8>::new();
    BalanceOfCommand { owner: USER_ADDR }.encode_for_send(&mut input);
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected = U256::from(deployer_1_2_transfer - deployer_2_1_transfer_from + amount_to_mint);
    let recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(expected, recovered);

    // after mint
    let mut input = Vec::<u8>::new();
    input.extend(SIG_TOTAL_SUPPLY.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(total_supply.add(U256::from(amount_to_mint)), recovered);
}
