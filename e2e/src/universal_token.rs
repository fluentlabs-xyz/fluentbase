use crate::EvmTestingContextWithGenesis;
use alloc::vec::Vec;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{
    address, bytes, hex, storage::StorageDescriptor, universal_token::*, Address, Bytes,
    ContractContextV1, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME, U256,
};
use fluentbase_testing::{EvmTestingContext, TxBuilder};
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

fn call_with_sig_no_funding(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
) -> Vec<u8> {
    let mut tx = TxBuilder::call(ctx, *caller, *callee, None)
        .input(input)
        .gas_price(0);
    let result = tx.exec();
    println!("result: {:?}", result);
    assert!(result.is_success());
    result.output().unwrap().to_vec()
}

fn call_with_sig_funding(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
    value: U256,
) -> Vec<u8> {
    let mut tx = TxBuilder::call(ctx, *caller, *callee, None)
        .input(input)
        .value(value)
        .gas_price(0);
    let result = tx.exec();
    println!("result: {:?}", result);
    assert!(result.is_success());
    result.output().unwrap().to_vec()
}

fn call_with_sig_revert_no_funding(
    ctx: &mut EvmTestingContext,
    input: Bytes,
    caller: &Address,
    callee: &Address,
) -> Bytes {
    let mut tx = TxBuilder::call(ctx, *caller, *callee, None)
        .input(input)
        .gas_price(0);
    let result = tx.exec();
    match result {
        ExecutionResult::Revert {
            gas_used: _,
            output,
        } => output,
        _ => panic!("expected revert, got: {:?}", &result),
    }
}

fn deploy_wrapped_ust20(
    ctx: &mut EvmTestingContext,
    deployer: Address,
    pauser: Address,
) -> Address {
    let init = InitialSettings {
        token_name: "Wrapped Ether".into(),
        token_symbol: "WETH".into(),
        decimals: 18,
        initial_supply: U256::ZERO,
        minter: Address::ZERO,
        pauser,
        wrapped: Some(true),
    }
    .encode_with_prefix();
    ctx.deploy_evm_tx(deployer, init)
}

fn deploy_wrapped_ust20_with_minter(
    ctx: &mut EvmTestingContext,
    deployer: Address,
    minter: Address,
    pauser: Address,
) -> Address {
    let init = InitialSettings {
        token_name: "Wrapped Ether".into(),
        token_symbol: "WETH".into(),
        decimals: 18,
        initial_supply: U256::ZERO,
        minter,
        pauser,
        wrapped: Some(true),
    }
    .encode_with_prefix();
    ctx.deploy_evm_tx(deployer, init)
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
        decimals: 18,
        initial_supply: U256::from(0xffff_ffffu64),
        minter: Address::ZERO,
        pauser: Address::ZERO,
        wrapped: None,
    };
    let total_supply = U256::from(0xffff_ffffu64);
    let amount_to_mint = 93842;

    let init_bytecode: Bytes = initial_settings.encode_with_prefix();
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDR, init_bytecode);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_ERC20_TOTAL_SUPPLY.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let total_supply_recovered = u256_from_slice_try(output_data.as_ref()).unwrap();
    assert_eq!(total_supply, total_supply_recovered);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_ERC20_PAUSE.to_be_bytes());
    let output = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_UST_NOT_PAUSABLE, evm_exit_code);

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
    assert_eq!(ERR_UST_NOT_MINTABLE, evm_exit_code);
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
        decimals: 18,
        initial_supply: U256::from(0xffff_ffffu64),
        minter: DEPLOYER_ADDR,
        pauser: DEPLOYER_ADDR,
        wrapped: None,
    };
    let total_supply = U256::from(0xffff_ffffu64);
    let token_name = "NaMe";
    let token_symbol = "SyMbOl";
    let decimals = U256::from(18);
    let deployer_1_2_transfer = 12345678;
    let deployer_2_1_allowance = 1234567;
    let deployer_2_1_transfer_from = 1234;
    let amount_to_mint = 93842;

    let init_bytecode = initial_settings.encode_with_prefix();
    let contract_address = ctx.deploy_evm_tx(DEPLOYER_ADDR, init_bytecode);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_ERC20_TOTAL_SUPPLY.to_be_bytes());
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
    input.extend(SIG_ERC20_NAME.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected_token_name = hex!("000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000044e614d6500000000000000000000000000000000000000000000000000000000");
    assert_eq!(&expected_token_name, output_data.as_slice());

    // SIG_SYMBOL
    let mut input = Vec::<u8>::new();
    input.extend(SIG_ERC20_SYMBOL.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let expected_token_symbol = hex!("0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000653794d624f6c0000000000000000000000000000000000000000000000000000");
    assert_eq!(&expected_token_symbol, output_data.as_slice());

    let mut input = Vec::<u8>::new();
    input.extend(SIG_ERC20_DECIMALS.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(decimals, recovered);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_ERC20_TOTAL_SUPPLY.to_be_bytes());
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
    assert_eq!(ERR_ERC20_INSUFFICIENT_ALLOWANCE, evm_exit_code);

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
    input.extend(SIG_ERC20_PAUSE.to_be_bytes());
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
    input.extend(SIG_ERC20_PAUSE.to_be_bytes());
    let output = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_PAUSABLE_ENFORCED_PAUSE, evm_exit_code);

    let mut input = Vec::<u8>::new();
    input.extend(SIG_ERC20_UNPAUSE.to_be_bytes());
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
    input.extend(SIG_ERC20_UNPAUSE.to_be_bytes());
    let output = call_with_sig_revert(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_PAUSABLE_EXPECTED_PAUSE, evm_exit_code);

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
    input.extend(SIG_ERC20_TOTAL_SUPPLY.to_be_bytes());
    let output_data = call_with_sig(
        &mut ctx,
        input.clone().into(),
        &DEPLOYER_ADDR,
        &contract_address,
    );
    let recovered = u256_from_slice_try(output_data.as_ref()).expect("output is not a u256 repr");
    assert_eq!(total_supply.add(U256::from(amount_to_mint)), recovered);
}

#[test]
fn reverted_transaction_should_not_commit_changes() {
    const ACC1_ADDRESS: Address = Address::with_last_byte(77);
    const ACC2_ADDRESS: Address = Address::with_last_byte(88);
    const ACC3_ADDRESS: Address = Address::with_last_byte(99);

    let mut ctx = EvmTestingContext::default().with_full_genesis();

    // Deploy an ERC20 token with max supply
    let initial_settings = InitialSettings {
        token_name: Default::default(),
        token_symbol: Default::default(),
        decimals: 0,
        initial_supply: U256::from(10),
        minter: ACC1_ADDRESS,
        pauser: Address::ZERO,
        wrapped: None,
    }
    .encode_with_prefix();
    let contract_address = ctx.deploy_evm_tx(ACC1_ADDRESS, initial_settings);
    ctx.sdk.context_mut().address = contract_address;

    // Check minter balance (should be U256::MAX)
    let mut input = Vec::new();
    BalanceOfCommand {
        owner: ACC1_ADDRESS,
    }
    .encode_for_send(&mut input);
    let result = ctx.call_evm_tx(ACC1_ADDRESS, contract_address, input.into(), None, None);
    assert!(result.is_success());
    let balance = U256::from_be_slice(result.into_output().unwrap_or_default().as_ref());
    assert_eq!(balance, U256::from(10));

    // Approve 1 to spender (spender balance is 0)
    let mut input = Vec::new();
    ApproveCommand {
        spender: ACC1_ADDRESS,
        amount: U256::ONE,
    }
    .encode_for_send(&mut input);
    let result = ctx.call_evm_tx(ACC2_ADDRESS, contract_address, input.into(), None, None);
    assert!(result.is_success());

    // Transfer from acc2 to acc3 (should fail with insufficient balance)
    let mut input = Vec::new();
    TransferFromCommand {
        from: ACC2_ADDRESS,
        to: ACC3_ADDRESS,
        amount: U256::ONE,
    }
    .encode_for_send(&mut input);
    let result = ctx.call_evm_tx(ACC1_ADDRESS, contract_address, input.into(), None, None);
    assert_eq!(ERR_ERC20_INSUFFICIENT_BALANCE, 0xe450d38c);
    assert!(!result.is_success());
    assert_eq!(
        result.output().unwrap_or_default().as_ref(),
        hex!("0x4e487b7100000000000000000000000000000000000000000000000000000000e450d38c")
    );

    // Allowance should not change
    let mut input = Vec::new();
    AllowanceCommand {
        owner: ACC2_ADDRESS,
        spender: ACC1_ADDRESS,
    }
    .encode_for_send(&mut input);
    let result = ctx.call_evm_tx(ACC1_ADDRESS, contract_address, input.into(), None, None);
    assert!(result.is_success());
    let balance = U256::from_be_slice(result.into_output().unwrap_or_default().as_ref());
    assert_eq!(balance, U256::ONE);
}

#[test]
fn invoke_ust20_transfer_multiple_times() {
    let mut ctx = EvmTestingContext::default().with_remote_genesis("v0.5.8");

    let mut initial_settings = InitialSettings {
        token_name: "NaMe".into(),
        token_symbol: "SyMbOl".into(),
        decimals: 18,
        initial_supply: U256::from(1000),
        minter: DEPLOYER_ADDR,
        pauser: DEPLOYER_ADDR,
        wrapped: None,
    }
    .encode_with_prefix();

    ctx.add_balance(DEPLOYER_ADDR, U256::from(100_000000000000000000u128));

    let repeat_transfer_address = ctx.deploy_evm_tx(
        DEPLOYER_ADDR,
        hex::decode(&include_bytes!("../assets/ERC20RepeatTransfer.bin"))
            .unwrap()
            .into(),
    );
    println!("callee contract address: {:?}", repeat_transfer_address);
    let ust20_address = ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings);
    println!("ust20 address: {:?}", ust20_address);

    sol! {
        function transfer(address to, uint256 amount) external;

        function balanceOf(address owner) external;

        function repeatTransfer(
            address token,
            address to,
            uint256 amount,
            uint256 times
        ) external;
    }

    let input = transferCall {
        to: repeat_transfer_address,
        amount: U256::from(1000),
    }
    .abi_encode();
    let result = ctx.call_evm_tx(DEPLOYER_ADDR, ust20_address, input.into(), None, None);
    assert!(result.is_success());
    println!("result: {:?}", result.gas_used());

    let input = balanceOfCall {
        owner: repeat_transfer_address,
    }
    .abi_encode();
    let result = ctx.call_evm_tx(DEPLOYER_ADDR, ust20_address, input.into(), None, None);
    assert!(result.is_success());

    let input = repeatTransferCall {
        token: ust20_address,
        to: Address::repeat_byte(3),
        amount: U256::from(1),
        times: U256::from(1000),
    }
    .abi_encode();
    let result = ctx.call_evm_tx(
        DEPLOYER_ADDR,
        repeat_transfer_address,
        input.into(),
        Some(10_000_000),
        None,
    );
    assert!(result.is_success());
    println!("result: {:?}", result.gas_used());
}

#[test]
fn test_ust20_wrapped_token_cant_be_mintable() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let input = bytes!("0x455243205772617070656420457468657200000000000000000000000000000000000000574554480000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000120000000000000000000000000000000000000000000000000000000000000000000000000000000000000000911d15011b3358344d2b4a9a01a20a16ff4274a2000000000000000000000000911d15011b3358344d2b4a9a01a20a16ff4274a20000000000000000000000000000000000000000000000000000000000000001");
    let result = TxBuilder::create(&mut ctx, Address::repeat_byte(0x11), input).exec();
    let output = match result {
        ExecutionResult::Revert { output, .. } => output,
        _ => panic!("unexpected return value: {:?}", result),
    };
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_UST_NOT_MINTABLE, evm_exit_code);
}

#[test]
fn wrapped_withdraw_transfers_native_and_updates_supply_consistently() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let deployer = Address::repeat_byte(0x11);
    let user = Address::repeat_byte(0x22);
    let token = deploy_wrapped_ust20(&mut ctx, deployer, deployer);

    let deposit = U256::from(100u64);
    let withdraw = U256::from(40u64);

    ctx.add_balance(user, U256::from(1_000u64));
    let user_before = ctx.get_balance(user);
    let token_before = ctx.get_balance(token);

    let mut deposit_tx = TxBuilder::call(&mut ctx, user, token, Some(deposit))
        .input(bytes!("0xd0e30db0"))
        .gas_price(0);
    let deposit_result = deposit_tx.exec();
    assert!(deposit_result.is_success());

    assert_eq!(ctx.get_balance(user), user_before - deposit);
    assert_eq!(ctx.get_balance(token), token_before + deposit);

    let mut input = Vec::new();
    WithdrawCommand { amount: withdraw }.encode_for_send(&mut input);
    let mut withdraw_tx = TxBuilder::call(&mut ctx, user, token, None)
        .input(input.into())
        .gas_price(0);
    let withdraw_result = withdraw_tx.exec();
    assert!(withdraw_result.is_success());

    assert_eq!(ctx.get_balance(user), user_before - deposit + withdraw);
    assert_eq!(ctx.get_balance(token), token_before + deposit - withdraw);

    let mut input = Vec::new();
    BalanceOfCommand { owner: user }.encode_for_send(&mut input);
    let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(
        u256_from_slice_try(output.as_ref()).unwrap(),
        deposit - withdraw
    );

    let mut input = Vec::new();
    input.extend(SIG_ERC20_TOTAL_SUPPLY.to_be_bytes());
    let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(
        u256_from_slice_try(output.as_ref()).unwrap(),
        deposit - withdraw
    );
}

#[test]
fn wrapped_withdraw_insufficient_balance_reverts_without_state_changes() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let deployer = Address::repeat_byte(0x11);
    let user = Address::repeat_byte(0x22);
    let token = deploy_wrapped_ust20(&mut ctx, deployer, deployer);

    let deposit = U256::from(7u64);
    let withdraw = U256::from(9u64);

    ctx.add_balance(user, U256::from(100u64));
    let mut deposit_tx = TxBuilder::call(&mut ctx, user, token, Some(deposit))
        .input(bytes!("0xd0e30db0"))
        .gas_price(0);
    assert!(deposit_tx.exec().is_success());

    let user_before = ctx.get_balance(user);
    let token_before = ctx.get_balance(token);

    let mut input = Vec::new();
    WithdrawCommand { amount: withdraw }.encode_for_send(&mut input);
    let output = call_with_sig_revert_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_ERC20_INSUFFICIENT_BALANCE, evm_exit_code);

    assert_eq!(ctx.get_balance(user), user_before);
    assert_eq!(ctx.get_balance(token), token_before);

    let mut input = Vec::new();
    BalanceOfCommand { owner: user }.encode_for_send(&mut input);
    let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(u256_from_slice_try(output.as_ref()).unwrap(), deposit);

    let mut input = Vec::new();
    input.extend(SIG_ERC20_TOTAL_SUPPLY.to_be_bytes());
    let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(u256_from_slice_try(output.as_ref()).unwrap(), deposit);
}

#[test]
fn wrapped_withdraw_when_paused_reverts_without_state_changes() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let deployer = Address::repeat_byte(0x11);
    let user = Address::repeat_byte(0x22);
    let token = deploy_wrapped_ust20(&mut ctx, deployer, deployer);

    let deposit = U256::from(11u64);
    ctx.add_balance(user, U256::from(100u64));
    let mut deposit_tx = TxBuilder::call(&mut ctx, user, token, Some(deposit))
        .input(bytes!("0xd0e30db0"))
        .gas_price(0);
    assert!(deposit_tx.exec().is_success());

    let mut pause = Vec::new();
    pause.extend(SIG_ERC20_PAUSE.to_be_bytes());
    let _ = call_with_sig_no_funding(&mut ctx, pause.into(), &deployer, &token);

    let user_before = ctx.get_balance(user);
    let token_before = ctx.get_balance(token);

    let mut input = Vec::new();
    WithdrawCommand {
        amount: U256::from(1u64),
    }
    .encode_for_send(&mut input);
    let output = call_with_sig_revert_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
    let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
    assert_eq!(ERR_PAUSABLE_ENFORCED_PAUSE, evm_exit_code);

    assert_eq!(ctx.get_balance(user), user_before);
    assert_eq!(ctx.get_balance(token), token_before);

    let mut input = Vec::new();
    BalanceOfCommand { owner: user }.encode_for_send(&mut input);
    let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(u256_from_slice_try(output.as_ref()).unwrap(), deposit);
}

#[test]
fn wrapped_withdraw_halts_on_native_balance_underflow_and_preserves_storage() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    let deployer = Address::repeat_byte(0x11);
    let user = Address::repeat_byte(0x22);
    let token = deploy_wrapped_ust20_with_minter(&mut ctx, deployer, Address::ZERO, deployer);

    ctx.add_balance(user, U256::from(1000));

    let minted = U256::from(100);

    let user_balance_before = ctx.get_balance(user);
    let mut deposit = Vec::new();
    DepositCommand {}.encode_for_send(&mut deposit);
    let _ = call_with_sig_funding(&mut ctx, deposit.into(), &user, &token, minted);
    assert_eq!(ctx.get_balance(token), minted);
    assert_eq!(user_balance_before - ctx.get_balance(user), minted);

    let mut input = Vec::new();
    WithdrawCommand {
        amount: U256::from(200),
    }
    .encode_for_send(&mut input);
    let mut tx = TxBuilder::call(&mut ctx, user, token, None)
        .input(input.into())
        .gas_price(0);
    let result = tx.exec();

    match result {
        ExecutionResult::Revert { output, .. } => {
            assert_eq!(output[32..36], ERR_ERC20_INSUFFICIENT_BALANCE.to_be_bytes());
        }
        _ => panic!("expected revert(0xe450d38c), got: {:?}", result),
    }

    let mut input = Vec::new();
    BalanceOfCommand { owner: user }.encode_for_send(&mut input);
    let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(u256_from_slice_try(output.as_ref()).unwrap(), minted);

    let mut input = Vec::new();
    input.extend(SIG_ERC20_TOTAL_SUPPLY.to_be_bytes());
    let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
    assert_eq!(u256_from_slice_try(output.as_ref()).unwrap(), minted);
}

// #[test]
// fn wrapped_withdraw_reverts_with_insufficient_balance_on_native_underflow() {
//     let mut ctx = EvmTestingContext::default().with_full_genesis();
//     let deployer = Address::repeat_byte(0x11);
//     let user = Address::repeat_byte(0x22);
//     let token = deploy_wrapped_ust20_with_minter(&mut ctx, deployer, deployer, deployer);
//
//     let minted = U256::from(9u64);
//
//     let mut mint = Vec::new();
//     MintCommand {
//         to: user,
//         amount: minted,
//     }
//     .encode_for_send(&mut mint);
//     let _ = call_with_sig_no_funding(&mut ctx, mint.into(), &deployer, &token);
//
//     assert_eq!(ctx.get_balance(token), U256::ZERO);
//
//     let mut input = Vec::new();
//     WithdrawCommand { amount: U256::ONE }.encode_for_send(&mut input);
//     let output = call_with_sig_revert_no_funding(&mut ctx, input.into(), &user, &token);
//     assert_eq!(output[0..4], [0x4e, 0x48, 0x7b, 0x71]);
//     let evm_exit_code = u32::from_be_bytes(output[32..].try_into().unwrap());
//     assert_eq!(ERR_ERC20_INSUFFICIENT_BALANCE, evm_exit_code);
//
//     let mut input = Vec::new();
//     BalanceOfCommand { owner: user }.encode_for_send(&mut input);
//     let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
//     assert_eq!(u256_from_slice_try(output.as_ref()).unwrap(), minted);
//
//     let mut input = Vec::new();
//     input.extend(SIG_ERC20_TOTAL_SUPPLY.to_be_bytes());
//     let output = call_with_sig_no_funding(&mut ctx, input.into(), &user, &token);
//     assert_eq!(u256_from_slice_try(output.as_ref()).unwrap(), minted);
// }
