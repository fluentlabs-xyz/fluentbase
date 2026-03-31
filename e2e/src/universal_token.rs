use crate::EvmTestingContextWithGenesis;
use alloc::vec::Vec;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{sol, SolCall};
use fluentbase_sdk::{
    address, bytes, crypto::crypto_keccak256, hex, storage::StorageDescriptor,
    universal_token::*, Address, Bytes, ContractContextV1, B256, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
    U256,
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

fn abi_word_addr(a: Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a.as_ref());
    w
}

fn abi_word_u256(x: U256) -> [u8; 32] {
    x.to_be_bytes::<{ U256::BYTES }>()
}

fn permit_digest(
    token_name: &str,
    chain_id: u64,
    verifying_contract: Address,
    owner: Address,
    spender: Address,
    value: U256,
    nonce: U256,
    deadline: U256,
) -> B256 {
    let domain_typehash = crypto_keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let permit_typehash = crypto_keccak256(
        "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)",
    );
    let version_hash = crypto_keccak256("1");
    let name_hash = crypto_keccak256(token_name.as_bytes());

    let mut encoded_domain = Vec::with_capacity(32 * 5);
    encoded_domain.extend_from_slice(domain_typehash.as_slice());
    encoded_domain.extend_from_slice(name_hash.as_slice());
    encoded_domain.extend_from_slice(version_hash.as_slice());
    encoded_domain.extend_from_slice(&abi_word_u256(U256::from(chain_id)));
    encoded_domain.extend_from_slice(&abi_word_addr(verifying_contract));
    let domain_separator = crypto_keccak256(encoded_domain);

    let mut encoded_permit = Vec::with_capacity(32 * 6);
    encoded_permit.extend_from_slice(permit_typehash.as_slice());
    encoded_permit.extend_from_slice(&abi_word_addr(owner));
    encoded_permit.extend_from_slice(&abi_word_addr(spender));
    encoded_permit.extend_from_slice(&abi_word_u256(value));
    encoded_permit.extend_from_slice(&abi_word_u256(nonce));
    encoded_permit.extend_from_slice(&abi_word_u256(deadline));
    let permit_hash = crypto_keccak256(encoded_permit);

    let mut digest_payload = Vec::with_capacity(66);
    digest_payload.extend_from_slice(b"\x19\x01");
    digest_payload.extend_from_slice(domain_separator.as_slice());
    digest_payload.extend_from_slice(permit_hash.as_slice());
    crypto_keccak256(digest_payload)
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

#[test]
fn universal_token_permit_sets_allowance_and_nonce() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer: PrivateKeySigner =
        "0xf0bc949485d112791637d7eb29dea3fd1e0758e8fea3ef542a4245bc896736cc"
            .parse()
            .unwrap();
    let owner = Address::from_slice(signer.address().as_ref());
    let spender = RECIPIENT_ADDR;

    let initial_settings = InitialSettings {
        token_name: "Token".into(),
        token_symbol: "TKN".into(),
        decimals: 18,
        initial_supply: U256::ZERO,
        minter: Address::ZERO,
        pauser: Address::ZERO,
    }
    .encode_with_prefix();

    let token = ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings);

    sol! {
        function permit(address owner,address spender,uint256 value,uint256 deadline,uint8 v,bytes32 r,bytes32 s) external;
        function allowance(address owner,address spender) external view returns (uint256);
        function nonces(address owner) external view returns (uint256);
        function DOMAIN_SEPARATOR() external view returns (bytes32);
    }

    let domain_out = call_with_sig(
        &mut ctx,
        DOMAIN_SEPARATORCall {}.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(domain_out.len(), 32);

    let chain_id = ctx.cfg.chain_id;
    let nonce = U256::ZERO;
    let value = U256::from(777u64);
    let deadline = U256::MAX;

    let digest = permit_digest(
        "Token", chain_id, token, owner, spender, value, nonce, deadline,
    );
    let sig = signer.sign_hash_sync(&digest).unwrap();
    let sig_bytes = sig.as_bytes();
    let r = B256::from_slice(&sig_bytes[0..32]);
    let s = B256::from_slice(&sig_bytes[32..64]);
    let v = sig_bytes[64];

    let input = permitCall {
        owner,
        spender,
        value,
        deadline,
        v,
        r,
        s,
    }
    .abi_encode();
    assert_eq!(input.len(), 228, "permit calldata must include selector");
    assert_eq!(
        u32::from_be_bytes(input[0..4].try_into().unwrap()),
        SIG_ERC20_PERMIT,
        "permit selector mismatch"
    );

    let result = ctx.call_evm_tx(DEPLOYER_ADDR, token, input.into(), None, None);
    assert!(result.is_success(), "permit failed: {result:?}");

    let allowance_out = call_with_sig(
        &mut ctx,
        allowanceCall { owner, spender }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(allowance_out.as_ref()), value);

    let nonces_out = call_with_sig(
        &mut ctx,
        noncesCall { owner }.abi_encode().into(),
        &DEPLOYER_ADDR,
        &token,
    );
    assert_eq!(U256::from_be_slice(nonces_out.as_ref()), U256::ONE);
}

#[test]
fn universal_token_permit_rejects_invalid_signature() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();

    let signer: PrivateKeySigner =
        "0xf0bc949485d112791637d7eb29dea3fd1e0758e8fea3ef542a4245bc896736cc"
            .parse()
            .unwrap();
    let owner = Address::from_slice(signer.address().as_ref());
    let spender = RECIPIENT_ADDR;

    let initial_settings = InitialSettings {
        token_name: "Token".into(),
        token_symbol: "TKN".into(),
        decimals: 18,
        initial_supply: U256::ZERO,
        minter: Address::ZERO,
        pauser: Address::ZERO,
    }
    .encode_with_prefix();

    let token = ctx.deploy_evm_tx(DEPLOYER_ADDR, initial_settings);

    sol! {
        function permit(address owner,address spender,uint256 value,uint256 deadline,uint8 v,bytes32 r,bytes32 s) external;
    }

    let chain_id = ctx.cfg.chain_id;
    let value = U256::from(111u64);
    let deadline = U256::MAX;

    // Sign digest for a different spender
    let digest = permit_digest(
        "Token",
        chain_id,
        token,
        owner,
        Address::with_last_byte(9),
        value,
        U256::ZERO,
        deadline,
    );
    let sig = signer.sign_hash_sync(&digest).unwrap();
    let sig_bytes = sig.as_bytes();
    let r = B256::from_slice(&sig_bytes[0..32]);
    let s = B256::from_slice(&sig_bytes[32..64]);
    let v = sig_bytes[64];

    let input = permitCall {
        owner,
        spender,
        value,
        deadline,
        v,
        r,
        s,
    }
    .abi_encode();

    let output = call_with_sig_revert(&mut ctx, input.into(), &DEPLOYER_ADDR, &token);
    assert!(
        output
            .as_ref()
            .ends_with(&ERR_UST_INVALID_SIGNATURE.to_be_bytes()),
        "unexpected revert payload: 0x{}",
        hex::encode(output.as_ref())
    );
}
