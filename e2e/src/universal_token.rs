use crate::helpers::call_with_sig;
use crate::EvmTestingContextWithGenesis;
use alloc::vec::Vec;
use fluentbase_sdk::{address, Address};
use fluentbase_sdk_testing::EvmTestingContext;
use fluentbase_svm::account::{AccountSharedData, ReadableAccount};
use fluentbase_svm::account_info::AccountInfo;
use fluentbase_svm::error::SvmError;
use fluentbase_svm::helpers::{
    serialize_svm_program_params_from_instruction, storage_read_account_data,
    storage_write_account_data,
};
use fluentbase_svm::pubkey::Pubkey;
use fluentbase_svm::solana_program::instruction::{AccountMeta, Instruction};
use fluentbase_svm::token_2022;
use fluentbase_svm::token_2022::helpers::{
    account_from_account_info, account_info_from_meta_and_account,
};
use fluentbase_svm::token_2022::instruction::initialize_account;
use fluentbase_svm::token_2022::instruction::initialize_mint;
use fluentbase_svm::token_2022::instruction::initialize_mint2;
use fluentbase_svm::token_2022::instruction::mint_to;
#[allow(deprecated)]
use fluentbase_svm::token_2022::instruction::transfer;
use fluentbase_svm::token_2022::instruction::transfer_checked;
use fluentbase_svm::token_2022::state::{Account, Mint};
use fluentbase_svm_common::common::{lamports_to_bytes, pubkey_from_evm_address};
use fluentbase_types::{ContractContextV1, ERC20_MAGIC_BYTES, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME};
use fluentbase_universal_token::common::sig_to_bytes;
use fluentbase_universal_token::consts::{
    SIG_BALANCE, SIG_BALANCE_OF, SIG_INITIALIZE_ACCOUNT, SIG_INITIALIZE_MINT, SIG_MINT_TO,
    SIG_TOKEN2022, SIG_TRANSFER, SIG_TRANSFER_FROM,
};
use solana_program_option::COption;
use solana_program_pack::Pack;

const USER_ADDRESS1: Address = address!("1111111111111111111111111111111111111111");
const USER_ADDRESS2: Address = address!("2222222222222222222222222222222222222222");
const USER_ADDRESS3: Address = address!("3333333333333333333333333333333333333333");
const USER_ADDRESS4: Address = address!("4444444444444444444444444444444444444444");
const USER_ADDRESS5: Address = address!("5555555555555555555555555555555555555555");
const USER_ADDRESS6: Address = address!("6666666666666666666666666666666666666666");
pub fn modify_account_info(
    ctx: &mut EvmTestingContext,
    pk: &Pubkey,
    f: impl FnOnce(&mut AccountInfo),
) {
    let account_meta = AccountMeta::default();
    let account1_data =
        storage_read_account_data(&ctx.sdk, &pk, Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME)).unwrap();
    let mut account: fluentbase_svm::account::Account = account1_data.into();
    let mut account_info = account_info_from_meta_and_account(&account_meta, &mut account);
    f(&mut account_info);
    let account1_data: AccountSharedData = account_from_account_info(&account_info).into();
    storage_write_account_data(
        &mut ctx.sdk,
        &pk,
        &account1_data,
        Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
    )
    .unwrap();
    ctx.commit_sdk_to_db();
}
pub fn modify_account_state(
    ctx: &mut EvmTestingContext,
    pk: &Pubkey,
    f: impl FnOnce(&mut Account),
) {
    modify_account_info(ctx, pk, |account_info| {
        let mut account1_state = Account::unpack_unchecked(&account_info.data.borrow()).unwrap();
        f(&mut account1_state);
        Account::pack(account1_state, &mut account_info.data.borrow_mut()).unwrap();
    });
}
pub fn build_input_raw(prefix: &[u8], instruction_data: &[u8]) -> Vec<u8> {
    let input = prefix
        .iter()
        .chain(instruction_data.iter())
        .copied()
        .collect();
    input
}
pub fn build_input(prefix: &[u8], instruction: &Instruction) -> Result<Vec<u8>, SvmError> {
    let mut input: Vec<u8> = prefix.to_vec();
    serialize_svm_program_params_from_instruction(&mut input, instruction)
        .expect("failed to serialize program params into init_bytecode");
    Ok(input)
}

#[test]
fn test_initialize_mint() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let program_id = token_2022::lib::id();
    let owner_key = Pubkey::new_unique();
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let mint2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let initialize_mint_instruction1 =
        initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap();
    let init_bytecode: Vec<u8> = build_input(&ERC20_MAGIC_BYTES, &initialize_mint_instruction1)
        .expect("failed to build input");
    let _contract_address = ctx.deploy_evm_tx(USER_ADDRESS1, init_bytecode.clone().into());

    // create another mint that can freeze
    let initialize_mint_instruction2 =
        initialize_mint(&program_id, &mint2_key, &owner_key, Some(&owner_key), 2).unwrap();
    let init_bytecode: Vec<u8> = build_input(&ERC20_MAGIC_BYTES, &initialize_mint_instruction2)
        .expect("failed to build input");
    let _contract_address = ctx.deploy_evm_tx(USER_ADDRESS2, init_bytecode.clone().into());

    ctx.commit_db_to_sdk();

    let mint_account = storage_read_account_data(
        &mut ctx.sdk,
        &mint_key,
        Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
    )
    .expect("failed to read initialized mint account");
    let mint1 = Mint::unpack_unchecked(&mint_account.data()).unwrap();
    assert_eq!(mint1.freeze_authority, COption::None);

    let mint2_account = storage_read_account_data(
        &mut ctx.sdk,
        &mint2_key,
        Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
    )
    .expect("failed to read initialized mint account");
    let mint2 = Mint::unpack_unchecked(&mint2_account.data()).unwrap();
    assert_eq!(mint2.freeze_authority, COption::Some(owner_key));
}

#[test]
fn test_initialize_mint2() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let program_id = token_2022::lib::id();
    let owner_key = Pubkey::new_unique();
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let mint2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let initialize_mint2_instruction1 =
        initialize_mint2(&program_id, &mint_key, &owner_key, None, 2).unwrap();
    let init_bytecode: Vec<u8> = build_input(&ERC20_MAGIC_BYTES, &initialize_mint2_instruction1)
        .expect("failed to build input");
    let _contract_address = ctx.deploy_evm_tx(USER_ADDRESS1, init_bytecode.clone().into());
    // try to create 2nd time
    assert!(ctx
        .deploy_evm_tx_result(USER_ADDRESS1, init_bytecode.clone().into())
        .is_err());

    // create another mint that can freeze
    let initialize_mint2_instruction2 =
        initialize_mint2(&program_id, &mint2_key, &owner_key, Some(&owner_key), 2).unwrap();
    let init_bytecode: Vec<u8> = build_input(&ERC20_MAGIC_BYTES, &initialize_mint2_instruction2)
        .expect("failed to build input");
    let _contract_address = ctx.deploy_evm_tx(USER_ADDRESS2, init_bytecode.clone().into());
    // try to create 2nd time
    assert!(ctx
        .deploy_evm_tx_result(USER_ADDRESS2, init_bytecode.clone().into())
        .is_err());

    ctx.commit_db_to_sdk();

    let mint_account = storage_read_account_data(
        &mut ctx.sdk,
        &mint_key,
        Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
    )
    .expect("failed to read initialized mint account");
    let mint1 = Mint::unpack_unchecked(&mint_account.data()).unwrap();
    assert_eq!(mint1.freeze_authority, COption::None);

    let mint2_account = storage_read_account_data(
        &mut ctx.sdk,
        &mint2_key,
        Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
    )
    .expect("failed to read initialized mint account");
    let mint2 = Mint::unpack_unchecked(&mint2_account.data()).unwrap();
    assert_eq!(mint2.freeze_authority, COption::Some(owner_key));
}

#[test]
fn test_initialize_mint_account() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let program_id = token_2022::lib::id();
    let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let account_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let initialize_mint_instruction =
        initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap();
    let init_bytecode = build_input(&ERC20_MAGIC_BYTES, &initialize_mint_instruction)
        .expect("failed to build input");
    let token_contract_address = ctx.deploy_evm_tx(USER_ADDRESS1, init_bytecode.clone().into());

    ctx.commit_db_to_sdk();

    let mint_account_after_deploy = storage_read_account_data(
        &mut ctx.sdk,
        &mint_key,
        Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
    )
    .expect("failed to read initialized mint account");

    let initialize_account_instruction1 =
        initialize_account(&program_id, &account_key, &mint_key, &owner_key).unwrap();
    let input = build_input(
        &sig_to_bytes(SIG_TOKEN2022),
        &initialize_account_instruction1,
    )
    .expect("failed to build input");
    let _output_data = call_with_sig(
        &mut ctx,
        input.into(),
        &USER_ADDRESS2,
        &token_contract_address,
    )
    .unwrap();

    ctx.commit_db_to_sdk();

    let mint_account_after_exec = storage_read_account_data(
        &mut ctx.sdk,
        &mint_key,
        Some(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME),
    )
    .expect("failed to read initialized mint account");
    assert_eq!(mint_account_after_deploy, mint_account_after_exec);
}

#[test]
fn test_transfer_dups() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let program_id = token_2022::lib::id();
    let account1_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let account2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let account3_key = pubkey_from_evm_address::<true>(&USER_ADDRESS3);
    let _account4_key = pubkey_from_evm_address::<true>(&USER_ADDRESS4);
    let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS5);
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS6);

    let initialize_mint_instruction =
        initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap();

    let init_bytecode = build_input(&ERC20_MAGIC_BYTES, &initialize_mint_instruction)
        .expect("failed to build input");
    let contract_address = ctx.deploy_evm_tx(USER_ADDRESS5, init_bytecode.clone().into());

    ctx.commit_db_to_sdk();

    let initialize_account1_instruction =
        initialize_account(&program_id, &account1_key, &mint_key, &account1_key).unwrap();
    let input = build_input(
        &sig_to_bytes(SIG_TOKEN2022),
        &initialize_account1_instruction,
    )
    .expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

    let initialize_account2_instruction =
        initialize_account(&program_id, &account2_key, &mint_key, &owner_key).unwrap();
    let input = build_input(
        &sig_to_bytes(SIG_TOKEN2022),
        &initialize_account2_instruction,
    )
    .expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();

    // mint to account
    let mint_to_instruction =
        mint_to(&program_id, &mint_key, &account1_key, &owner_key, &[], 1000).unwrap();
    let input = build_input(&sig_to_bytes(SIG_TOKEN2022), &mint_to_instruction)
        .expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();

    // source-owner transfer
    #[allow(deprecated)]
    let transfer_instruction = transfer(
        &program_id,
        &account1_key,
        &account2_key,
        &account1_key,
        &[],
        500,
    )
    .unwrap();
    let input = build_input(&sig_to_bytes(SIG_TOKEN2022), &transfer_instruction)
        .expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

    // source-owner TransferChecked
    let transfer_instruction = transfer_checked(
        &program_id,
        &account1_key,
        &mint_key,
        &account2_key,
        &account1_key,
        &[],
        500,
        2,
    )
    .unwrap();
    let input = build_input(&sig_to_bytes(SIG_TOKEN2022), &transfer_instruction)
        .expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

    ctx.commit_db_to_sdk();

    // source-delegate transfer
    modify_account_state(&mut ctx, &account1_key, |account_state| {
        account_state.amount = 1000;
        account_state.delegated_amount = 1000;
        account_state.delegate = COption::Some(account1_key);
        account_state.owner = owner_key;
    });
    #[allow(deprecated)]
    let instruction = transfer(
        &program_id,
        &account1_key,
        &account2_key,
        &account1_key,
        &[],
        500,
    )
    .unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

    // source-delegate TransferChecked
    #[allow(deprecated)]
    let transfer_checked_instruction = transfer_checked(
        &program_id,
        &account1_key,
        &mint_key,
        &account2_key,
        &account1_key,
        &[],
        500,
        2,
    )
    .unwrap();
    let input = build_input(&sig_to_bytes(SIG_TOKEN2022), &transfer_checked_instruction)
        .expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

    // test destination-owner transfer
    #[allow(deprecated)]
    let instruction =
        initialize_account(&program_id, &account3_key, &mint_key, &account2_key).unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    #[allow(deprecated)]
    let instruction =
        mint_to(&program_id, &mint_key, &account3_key, &owner_key, &[], 1000).unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();

    modify_account_info(&mut ctx, &account1_key, |account1_info| {
        account1_info.is_signer = false;
    });
    modify_account_info(&mut ctx, &account2_key, |account2_info| {
        account2_info.is_signer = true;
    });
    #[allow(deprecated)]
    let instruction = transfer(
        &program_id,
        &account3_key,
        &account2_key,
        &account2_key,
        &[],
        500,
    )
    .unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();

    // balance_of (ABI version)
    let mut input_data = vec![];
    input_data.extend_from_slice(account2_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE_OF), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1500);

    // destination-owner TransferChecked
    let instruction = transfer_checked(
        &program_id,
        &account3_key,
        &mint_key,
        &account2_key,
        &account2_key,
        &[],
        100,
        2,
    )
    .unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();

    // balance_of (ABI version)
    let mut input_data = vec![];
    input_data.extend_from_slice(account2_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE_OF), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1600);

    // transfer_from (ABI version)
    let amount: u64 = 100;
    let mut input_data = vec![];
    // from, to, authority, amount
    input_data.extend_from_slice(account3_key.as_ref());
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(&amount.to_be_bytes());
    let input = build_input_raw(&sig_to_bytes(SIG_TRANSFER_FROM), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // balance_of (ABI version)
    let mut input_data = vec![];
    input_data.extend_from_slice(account2_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE_OF), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1700);

    // balance (ABI version)
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1700);

    // transfer_from (ABI version)
    let amount: u64 = 100;
    let mut input_data = vec![];
    // from, to, authority, amount
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(account1_key.as_ref());
    input_data.extend_from_slice(owner_key.as_ref());
    input_data.extend_from_slice(&amount.to_be_bytes());
    let input = build_input_raw(&sig_to_bytes(SIG_TRANSFER_FROM), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // balance (ABI version)
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1600);

    // transfer (ABI version)
    let amount: u64 = 100;
    let mut input_data = vec![];
    // to, authority, amount
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(account1_key.as_ref());
    input_data.extend_from_slice(&amount.to_be_bytes());
    let input = build_input_raw(&sig_to_bytes(SIG_TRANSFER), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // balance (ABI version)
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1700);
}

#[test]
fn test_transfer_dups_abi_version() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let program_id = token_2022::lib::id();
    let account1_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let account2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let account3_key = pubkey_from_evm_address::<true>(&USER_ADDRESS3);
    let _account4_key = pubkey_from_evm_address::<true>(&USER_ADDRESS4);
    let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS5);
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS6);

    // initialize_mint (ABI version)
    let decimals: u8 = 2;
    let mut input_data = vec![];
    // mint, owner, freeze, decimals
    input_data.extend_from_slice(mint_key.as_ref());
    input_data.extend_from_slice(owner_key.as_ref());
    input_data.extend_from_slice(Pubkey::default().as_ref());
    input_data.push(decimals);
    let sig_bytes = sig_to_bytes(SIG_INITIALIZE_MINT);
    let input = build_input_raw(&sig_bytes, &input_data);
    let input = build_input_raw(&ERC20_MAGIC_BYTES, &input);
    let contract_address = ctx.deploy_evm_tx(USER_ADDRESS5, input.into());

    ctx.commit_db_to_sdk();

    // initialize_account1
    let mut input_data = vec![];
    // account, mint, owner
    input_data.extend_from_slice(account1_key.as_ref());
    input_data.extend_from_slice(mint_key.as_ref());
    input_data.extend_from_slice(account1_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // initialize_account2
    let mut input_data = vec![];
    // account, mint, owner
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(mint_key.as_ref());
    input_data.extend_from_slice(owner_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // mint_to
    let mut input_data = vec![];
    // mint, account, owner, amount
    input_data.extend_from_slice(mint_key.as_ref());
    input_data.extend_from_slice(account1_key.as_ref());
    input_data.extend_from_slice(owner_key.as_ref());
    input_data.extend_from_slice(&lamports_to_bytes(1000));
    let input = build_input_raw(&sig_to_bytes(SIG_MINT_TO), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // source-owner transfer
    #[allow(deprecated)]
    let transfer_instruction = transfer(
        &program_id,
        &account1_key,
        &account2_key,
        &account1_key,
        &[],
        500,
    )
    .unwrap();
    let input = build_input(&sig_to_bytes(SIG_TOKEN2022), &transfer_instruction)
        .expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

    // balance
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 500);

    ctx.commit_db_to_sdk();

    // source-delegate transfer
    modify_account_state(&mut ctx, &account1_key, |account_state| {
        account_state.amount = 1000;
        account_state.delegated_amount = 1000;
        account_state.delegate = COption::Some(account1_key);
        account_state.owner = owner_key;
    });
    #[allow(deprecated)]
    let instruction = transfer(
        &program_id,
        &account1_key,
        &account2_key,
        &account1_key,
        &[],
        500,
    )
    .unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

    // balance
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1000);

    // test destination-owner transfer
    // initialize_account3
    let mut input_data = vec![];
    // account, mint, owner
    input_data.extend_from_slice(account3_key.as_ref());
    input_data.extend_from_slice(mint_key.as_ref());
    input_data.extend_from_slice(account2_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // mint_to
    let mut input_data = vec![];
    // mint, account, owner, amount
    input_data.extend_from_slice(mint_key.as_ref());
    input_data.extend_from_slice(account3_key.as_ref());
    input_data.extend_from_slice(owner_key.as_ref());
    input_data.extend_from_slice(&lamports_to_bytes(1000));
    let input = build_input_raw(&sig_to_bytes(SIG_MINT_TO), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    modify_account_info(&mut ctx, &account1_key, |account1_info| {
        account1_info.is_signer = false;
    });
    modify_account_info(&mut ctx, &account2_key, |account2_info| {
        account2_info.is_signer = true;
    });
    #[allow(deprecated)]
    let instruction = transfer(
        &program_id,
        &account3_key,
        &account2_key,
        &account2_key,
        &[],
        500,
    )
    .unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();

    // balance_of
    let mut input_data = vec![];
    input_data.extend_from_slice(account2_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE_OF), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1000);

    // destination-owner TransferChecked
    let instruction = transfer_checked(
        &program_id,
        &account3_key,
        &mint_key,
        &account2_key,
        &account2_key,
        &[],
        100,
        2,
    )
    .unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();

    // balance_of
    let mut input_data = vec![];
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1100);

    // transfer_from
    let amount: u64 = 100;
    let mut input_data = vec![];
    // from, to, authority, amount
    input_data.extend_from_slice(account3_key.as_ref());
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(&amount.to_be_bytes());
    let input = build_input_raw(&sig_to_bytes(SIG_TRANSFER_FROM), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // balance_of
    let mut input_data = vec![];
    input_data.extend_from_slice(account2_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE_OF), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1200);

    // balance
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1200);

    // transfer_from
    let amount: u64 = 100;
    let mut input_data = vec![];
    // from, to, authority, amount
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(account1_key.as_ref());
    input_data.extend_from_slice(owner_key.as_ref());
    input_data.extend_from_slice(&amount.to_be_bytes());
    let input = build_input_raw(&sig_to_bytes(SIG_TRANSFER_FROM), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // balance
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1100);

    // transfer
    let amount: u64 = 100;
    let mut input_data = vec![];
    // to, authority, amount
    input_data.extend_from_slice(account2_key.as_ref());
    input_data.extend_from_slice(account1_key.as_ref());
    input_data.extend_from_slice(&amount.to_be_bytes());
    let input = build_input_raw(&sig_to_bytes(SIG_TRANSFER), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // balance
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1200);
}
