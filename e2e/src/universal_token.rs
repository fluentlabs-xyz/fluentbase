use crate::helpers::{
    call_with_sig, with_svm_account_info_mut, with_svm_account_mut, with_svm_account_state_mut,
};
use crate::EvmTestingContextWithGenesis;
use alloc::vec::Vec;
use fluentbase_sdk::Address;
use fluentbase_svm::account::ReadableAccount;
use fluentbase_svm::error::SvmError;
use fluentbase_svm::helpers::{
    serialize_svm_program_params_from_instruction, storage_read_account_data,
};
use fluentbase_svm::pubkey::Pubkey;
use fluentbase_svm::solana_program::instruction::Instruction;
use fluentbase_svm::token_2022;
use fluentbase_svm::token_2022::instruction::initialize_mint;
use fluentbase_svm::token_2022::instruction::initialize_mint2;
use fluentbase_svm::token_2022::instruction::mint_to;
#[allow(deprecated)]
use fluentbase_svm::token_2022::instruction::transfer;
use fluentbase_svm::token_2022::instruction::transfer_checked;
use fluentbase_svm::token_2022::instruction::{initialize_account, AuthorityType};
use fluentbase_svm::token_2022::state::{Account, AccountState, Mint};
use fluentbase_svm_common::common::{lamports_try_from_slice, pubkey_from_evm_address};
use fluentbase_svm_common::universal_token::{
    AllowanceParams, ApproveCheckedParams, ApproveParams, BurnCheckedParams, BurnParams,
    CloseAccountParams, FreezeAccountParams, InitializeAccountParams, InitializeMintParams,
    MintToParams, RevokeParams, SetAuthorityParams, ThawAccountParams, TransferFromParams,
    TransferParams,
};
use fluentbase_testing::{utf8_to_bytes, EvmTestingContext};
use fluentbase_types::{ContractContextV1, ERC20_MAGIC_BYTES, PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME};
use fluentbase_universal_token::common::sig_to_bytes;
use fluentbase_universal_token::consts::{
    SIG_ALLOWANCE, SIG_APPROVE, SIG_APPROVE_CHECKED, SIG_BALANCE, SIG_BALANCE_OF, SIG_BURN,
    SIG_BURN_CHECKED, SIG_CLOSE_ACCOUNT, SIG_DECIMALS, SIG_DECIMALS_FOR_MINT, SIG_FREEZE_ACCOUNT,
    SIG_INITIALIZE_ACCOUNT, SIG_INITIALIZE_MINT, SIG_MINT_TO, SIG_REVOKE, SIG_SET_AUTHORITY,
    SIG_THAW_ACCOUNT, SIG_TOKEN2022, SIG_TRANSFER, SIG_TRANSFER_FROM,
};
use solana_program_option::COption;
use solana_program_pack::Pack;

const USER_ADDRESS1: Address = Address::repeat_byte(0x1);
const USER_ADDRESS2: Address = Address::repeat_byte(0x2);
const USER_ADDRESS3: Address = Address::repeat_byte(0x3);
const USER_ADDRESS4: Address = Address::repeat_byte(0x4);
const USER_ADDRESS5: Address = Address::repeat_byte(0x5);
const USER_ADDRESS6: Address = Address::repeat_byte(0x6);
const USER_ADDRESS7: Address = Address::repeat_byte(0x7);
const USER_ADDRESS8: Address = Address::repeat_byte(0x8);
const USER_ADDRESS9: Address = Address::repeat_byte(0x9);
const USER_ADDRESS10: Address = Address::repeat_byte(0xa);
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

fn account_minimum_balance() -> u64 {
    // Rent::default().minimum_balance(Account::get_packed_len())
    0
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

    let decimals = 2;

    let initialize_mint_instruction =
        initialize_mint(&program_id, &mint_key, &owner_key, None, decimals).unwrap();
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
        decimals,
    )
    .unwrap();
    let input = build_input(&sig_to_bytes(SIG_TOKEN2022), &transfer_instruction)
        .expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();

    ctx.commit_db_to_sdk();

    // source-delegate transfer
    with_svm_account_state_mut(&mut ctx, &account1_key, |account_state| {
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
        decimals,
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

    with_svm_account_info_mut(&mut ctx, &account1_key, |account1_info| {
        account1_info.is_signer = false;
    });
    with_svm_account_info_mut(&mut ctx, &account2_key, |account2_info| {
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
    assert_eq!(balance, 2500);

    // destination-owner TransferChecked
    let instruction = transfer_checked(
        &program_id,
        &account3_key,
        &mint_key,
        &account2_key,
        &account2_key,
        &[],
        100,
        decimals,
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
    assert_eq!(balance, 2600);
}

#[test]
fn test_transfer_dups_abi() {
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

    // initialize_mint
    let decimals: u8 = 2;
    let mut input_data = vec![];
    InitializeMintParams {
        mint: &mint_key,
        mint_authority: &owner_key,
        freeze_opt: None,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_MINT), &input_data);
    let input = build_input_raw(&ERC20_MAGIC_BYTES, &input);
    let contract_address = ctx.deploy_evm_tx(USER_ADDRESS5, input.into());

    ctx.commit_db_to_sdk();

    // decimals for mint
    let mut input_data = vec![];
    input_data.extend_from_slice(mint_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_DECIMALS_FOR_MINT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], decimals);

    // initialize_account1
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account1_key,
        mint: &mint_key,
        owner: &account1_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // decimals for account
    let mut input_data = vec![];
    input_data.extend_from_slice(account1_key.as_ref());
    let input = build_input_raw(&sig_to_bytes(SIG_DECIMALS), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data, vec![decimals]);

    // initialize_account2
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account2_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // mint_to
    let amount = 1000;
    let mut input_data = vec![];
    MintToParams {
        mint: &mint_key,
        account: &account1_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
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
    with_svm_account_state_mut(&mut ctx, &account1_key, |account_state| {
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
    InitializeAccountParams {
        account: &account3_key,
        mint: &mint_key,
        owner: &account2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // mint_to
    let amount = 1000;
    let mut input_data = vec![];
    MintToParams {
        mint: &mint_key,
        account: &account3_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_MINT_TO), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    with_svm_account_info_mut(&mut ctx, &account1_key, |account1_info| {
        account1_info.is_signer = false;
    });
    with_svm_account_info_mut(&mut ctx, &account2_key, |account2_info| {
        account2_info.is_signer = true;
    });
    let amount: u64 = 500;
    let mut input_data = vec![];
    TransferFromParams {
        from: &account3_key,
        mint: &mint_key,
        to: &account2_key,
        authority: &account2_key,
        amount: &amount,
        decimals,
    }
    .serialize_into(&mut input_data);
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
        decimals,
    )
    .unwrap();
    let input =
        build_input(&sig_to_bytes(SIG_TOKEN2022), &instruction).expect("failed to build input");
    let _output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();

    // balance
    let input_data = vec![];
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1600);

    // transfer_from
    let amount: u64 = 100;
    let mut input_data = vec![];
    TransferFromParams {
        from: &account3_key,
        mint: &mint_key,
        to: &account2_key,
        authority: &account2_key,
        amount: &amount,
        decimals,
    }
    .serialize_into(&mut input_data);
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
    assert_eq!(balance, 1700);

    // balance
    let input = build_input_raw(&sig_to_bytes(SIG_BALANCE), &[]);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS2, &contract_address).unwrap();
    assert_eq!(output_data.len(), size_of::<u64>());
    let balance = u64::from_be_bytes(output_data.as_slice().try_into().unwrap());
    assert_eq!(balance, 1700);

    // transfer_from
    let amount: u64 = 100;
    let mut input_data = vec![];
    TransferFromParams {
        from: &account2_key,
        mint: &mint_key,
        to: &account1_key,
        authority: &owner_key,
        amount: &amount,
        decimals,
    }
    .serialize_into(&mut input_data);
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
    assert_eq!(balance, 1600);

    // transfer
    let amount: u64 = 100;
    let mut input_data = vec![];
    TransferParams {
        mint: &mint_key,
        to: &account2_key,
        authority: &account1_key,
        amount,
        decimals,
    }
    .serialize_into(&mut input_data);
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
    assert_eq!(balance, 1700);
}

#[test]
fn test_approve_abi() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let account_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let account2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let delegate_key = pubkey_from_evm_address::<true>(&USER_ADDRESS3);
    let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS4);
    let owner2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS5);
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS6);

    // initialize mint
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap(),
    //     vec![&mut mint_account /*&mut rent_sysvar*/],
    // )
    //     .unwrap();
    let incorrect_decimals = 0;
    let decimals = 2;
    let mut input_data = vec![];
    InitializeMintParams {
        mint: &mint_key,
        mint_authority: &owner_key,
        freeze_opt: None,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_MINT), &input_data);
    let input = build_input_raw(&ERC20_MAGIC_BYTES, &input);
    let contract_address = ctx.deploy_evm_tx(USER_ADDRESS5, input.into());

    // create account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // create another account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account2_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account2_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account2_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS1, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // // mint to account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     mint_to(&program_id, &mint_key, &account_key, &owner_key, &[], 1000).unwrap(),
    //     vec![&mut mint_account, &mut account_account, &mut owner_account],
    // )
    // .unwrap();
    let amount = 1000;
    let mut input_data = vec![];
    MintToParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_MINT_TO), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address).unwrap();
    assert_eq!(output_data.len(), 1);
    assert_eq!(output_data[0], 1);

    // // missing signer
    // let mut instruction = approve(
    //     &program_id,
    //     &account_key,
    //     &delegate_key,
    //     &owner_key,
    //     &[],
    //     100,
    // )
    // .unwrap();
    // instruction.accounts[2].is_signer = false;
    // assert_eq!(
    //     Err(ProgramError::MissingRequiredSignature),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         instruction,
    //         vec![
    //             &mut account_account,
    //             &mut delegate_account,
    //             &mut owner_account,
    //         ],
    //     )
    // );

    // // no owner
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         approve(
    //             &program_id,
    //             &account_key,
    //             &delegate_key,
    //             &owner2_key,
    //             &[],
    //             100
    //         )
    //         .unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut delegate_account,
    //             &mut owner2_account,
    //         ],
    //     )
    // );
    let amount = 100;
    let mut input_data = vec![];
    ApproveParams {
        source: &account_key,
        delegate: &delegate_key,
        owner: &owner2_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_APPROVE), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    let mut input_data = vec![];
    AllowanceParams {
        source: &account_key,
        delegate: &delegate_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_ALLOWANCE), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address).unwrap();
    assert_eq!(result.len(), size_of::<u64>());
    let allowance = lamports_try_from_slice(&result).expect("allowance bytes");
    assert_eq!(allowance, 0);

    // // approve delegate
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     approve(
    //         &program_id,
    //         &account_key,
    //         &delegate_key,
    //         &owner_key,
    //         &[],
    //         100,
    //     )
    //     .unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut delegate_account,
    //         &mut owner_account,
    //     ],
    // )
    // .unwrap();
    let amount = 100;
    let mut input_data = vec![];
    ApproveParams {
        source: &account_key,
        delegate: &delegate_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_APPROVE), &input_data);
    let output_data =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address).unwrap();
    assert_eq!(output_data, vec![1]);

    let mut input_data = vec![];
    AllowanceParams {
        source: &account_key,
        delegate: &delegate_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_ALLOWANCE), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address).unwrap();
    assert_eq!(result.len(), size_of::<u64>());
    let allowance = lamports_try_from_slice(&result).expect("allowance bytes");
    assert_eq!(allowance, amount);

    // // approve delegate 2, with incorrect decimals
    // assert_eq!(
    //     Err(TokenError::MintDecimalsMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         approve_checked(
    //             &program_id,
    //             &account_key,
    //             &mint_key,
    //             &delegate_key,
    //             &owner_key,
    //             &[],
    //             100,
    //             0 // <-- incorrect decimals
    //         )
    //         .unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut mint_account,
    //             &mut delegate_account,
    //             &mut owner_account,
    //         ],
    //     )
    // );
    let amount = 100;
    let mut input_data = vec![];
    ApproveCheckedParams {
        source: &account_key,
        mint: &mint_key,
        delegate: &delegate_key,
        owner: &owner_key,
        amount: &amount,
        decimals: incorrect_decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_APPROVE_CHECKED), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(18)"));

    // // approve delegate 2, with incorrect mint
    // assert_eq!(
    //     Err(TokenError::MintMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         approve_checked(
    //             &program_id,
    //             &account_key,
    //             &account2_key, // <-- bad mint
    //             &delegate_key,
    //             &owner_key,
    //             &[],
    //             100,
    //             0
    //         )
    //         .unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut account2_account, // <-- bad mint
    //             &mut delegate_account,
    //             &mut owner_account,
    //         ],
    //     )
    // );
    let amount = 100;
    let mut input_data = vec![];
    ApproveCheckedParams {
        source: &account_key,
        mint: &account2_key, // bad mint
        delegate: &delegate_key,
        owner: &owner_key,
        amount: &amount,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_APPROVE_CHECKED), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(3)"));

    let mut input_data = vec![];
    AllowanceParams {
        source: &account_key,
        delegate: &delegate_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_ALLOWANCE), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address).unwrap();
    assert_eq!(result.len(), size_of::<u64>());
    let allowance = lamports_try_from_slice(&result).expect("allowance bytes");
    assert_eq!(allowance, amount);

    // // approve delegate 2
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     approve_checked(
    //         &program_id,
    //         &account_key,
    //         &mint_key,
    //         &delegate_key,
    //         &owner_key,
    //         &[],
    //         100,
    //         decimals,
    //     )
    //     .unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut delegate_account,
    //         &mut owner_account,
    //     ],
    // )
    // .unwrap();
    let amount = 100;
    let mut input_data = vec![];
    ApproveCheckedParams {
        source: &account_key,
        mint: &mint_key,
        delegate: &delegate_key,
        owner: &owner_key,
        amount: &amount,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_APPROVE_CHECKED), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address).unwrap();
    assert_eq!(result, vec![1]);

    let mut input_data = vec![];
    AllowanceParams {
        source: &account_key,
        delegate: &delegate_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_ALLOWANCE), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address).unwrap();
    assert_eq!(result.len(), size_of::<u64>());
    let allowance = lamports_try_from_slice(&result).expect("allowance bytes");
    assert_eq!(allowance, amount);

    // // revoke delegate
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     revoke(&program_id, &account_key, &owner_key, &[]).unwrap(),
    //     vec![&mut account_account, &mut owner_account],
    // )
    // .unwrap();
    let mut input_data = vec![];
    RevokeParams {
        source: &account_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_REVOKE), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // approve delegate 3
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     approve_checked(
    //         &program_id,
    //         &account_key,
    //         &mint_key,
    //         &delegate_key,
    //         &owner_key,
    //         &[],
    //         100,
    //         decimals,
    //     )
    //     .unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut delegate_account,
    //         &mut owner_account,
    //     ],
    // )
    // .unwrap();
    let amount = 100;
    let mut input_data = vec![];
    ApproveCheckedParams {
        source: &account_key,
        mint: &mint_key,
        delegate: &delegate_key,
        owner: &owner_key,
        amount: &amount,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_APPROVE_CHECKED), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // revoke by delegate
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     revoke(&program_id, &account_key, &delegate_key, &[]).unwrap(),
    //     vec![&mut account_account, &mut delegate_account],
    // )
    // .unwrap();
    let mut input_data = vec![];
    RevokeParams {
        source: &account_key,
        owner: &delegate_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_REVOKE), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // fails the second time
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         revoke(&program_id, &account_key, &delegate_key, &[]).unwrap(),
    //         vec![&mut account_account, &mut delegate_account],
    //     )
    // );
    let mut input_data = vec![];
    RevokeParams {
        source: &account_key,
        owner: &delegate_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_REVOKE), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));
}

#[test]
fn test_set_authority_abi() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let account_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let account2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS3);
    let owner2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS4);
    let owner3_key = pubkey_from_evm_address::<true>(&USER_ADDRESS5);
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS6);
    let mint2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS7);

    // // create new mint with owner
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap(),
    //     vec![&mut mint_account /*&mut rent_sysvar*/],
    // )
    //     .unwrap();
    let decimals = 2;
    let mut input_data = vec![];
    InitializeMintParams {
        mint: &mint_key,
        mint_authority: &owner_key,
        freeze_opt: None,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_MINT), &input_data);
    let input = build_input_raw(&ERC20_MAGIC_BYTES, &input);
    let contract_address = ctx.deploy_evm_tx(USER_ADDRESS6, input.into());

    // // create mint with owner and freeze_authority
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_mint(&program_id, &mint2_key, &owner_key, Some(&owner_key), 2).unwrap(),
    //     vec![&mut mint2_account /*&mut rent_sysvar*/],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    InitializeMintParams {
        mint: &mint2_key,
        mint_authority: &owner_key,
        freeze_opt: Some(&owner2_key),
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_MINT), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS7, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // invalid account
    // assert_eq!(
    //     Err(ProgramError::InvalidAccountData),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         set_authority(
    //             &program_id,
    //             &account_key,
    //             Some(&owner2_key),
    //             AuthorityType::AccountOwner,
    //             &owner_key,
    //             &[]
    //         )
    //             .unwrap(),
    //         vec![&mut account_account, &mut owner_account],
    //     )
    // );
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::AccountOwner as u8,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(
        result.1,
        utf8_to_bytes("failed to process: InvalidAccountData")
    );

    // // create account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // create another account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account2_key, &mint2_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account2_account,
    //         &mut mint2_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account2_key,
        mint: &mint2_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // missing owner
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         set_authority(
    //             &program_id,
    //             &account_key,
    //             Some(&owner_key),
    //             AuthorityType::AccountOwner,
    //             &owner2_key,
    //             &[]
    //         )
    //             .unwrap(),
    //         vec![&mut account_account, &mut owner2_account],
    //     )
    // );
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: Some(&owner_key),
        authority_type: AuthorityType::AccountOwner as u8,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    // // owner did not sign
    // let mut instruction = set_authority(
    //     &program_id,
    //     &account_key,
    //     Some(&owner2_key),
    //     AuthorityType::AccountOwner,
    //     &owner_key,
    //     &[],
    // )
    //     .unwrap();
    // instruction.accounts[1].is_signer = false;
    // assert_eq!(
    //     Err(ProgramError::MissingRequiredSignature),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         instruction,
    //         vec![&mut account_account, &mut owner_account,],
    //     )
    // );
    //
    // // wrong authority type
    // assert_eq!(
    //     Err(TokenError::AuthorityTypeNotSupported.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         set_authority(
    //             &program_id,
    //             &account_key,
    //             Some(&owner2_key),
    //             AuthorityType::FreezeAccount,
    //             &owner_key,
    //             &[],
    //         )
    //             .unwrap(),
    //         vec![&mut account_account, &mut owner_account],
    //     )
    // );
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::FreezeAccount as u8,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(15)"));

    // // account owner may not be set to None
    // assert_eq!(
    //     Err(TokenError::InvalidInstruction.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         set_authority(
    //             &program_id,
    //             &account_key,
    //             None,
    //             AuthorityType::AccountOwner,
    //             &owner_key,
    //             &[],
    //         )
    //             .unwrap(),
    //         vec![&mut account_account, &mut owner_account],
    //     )
    // );
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: None,
        authority_type: AuthorityType::AccountOwner as u8,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(12)"));

    // // set delegate
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     approve(
    //         &program_id,
    //         &account_key,
    //         &owner2_key,
    //         &owner_key,
    //         &[],
    //         u64::MAX,
    //     )
    //         .unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut owner2_account,
    //         &mut owner_account,
    //     ],
    // )
    //     .unwrap();
    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.delegate, COption::Some(owner2_key));
    // assert_eq!(account.delegated_amount, u64::MAX);
    let amount = u64::MAX;
    let mut input_data = vec![];
    ApproveParams {
        source: &account_key,
        delegate: &owner2_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_APPROVE), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.delegate, COption::Some(owner2_key));
        assert_eq!(account.delegated_amount, u64::MAX);
    });

    // // set owner
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &account_key,
    //         Some(&owner3_key),
    //         AuthorityType::AccountOwner,
    //         &owner_key,
    //         &[],
    //     )
    //         .unwrap(),
    //     vec![&mut account_account, &mut owner_account],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: Some(&owner3_key),
        authority_type: AuthorityType::AccountOwner as u8,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // check delegate cleared
    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.delegate, COption::None);
    // assert_eq!(account.delegated_amount, 0);
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.delegate, COption::None);
        assert_eq!(account.delegated_amount, 0);
    });

    // // set owner without existing delegate
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &account_key,
    //         Some(&owner2_key),
    //         AuthorityType::AccountOwner,
    //         &owner3_key,
    //         &[],
    //     )
    //         .unwrap(),
    //     vec![&mut account_account, &mut owner3_account],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::AccountOwner as u8,
        owner: &owner3_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // set close_authority
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &account_key,
    //         Some(&owner2_key),
    //         AuthorityType::CloseAccount,
    //         &owner2_key,
    //         &[],
    //     )
    //         .unwrap(),
    //     vec![&mut account_account, &mut owner2_account],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::CloseAccount as u8,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // close_authority may be set to None
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &account_key,
    //         None,
    //         AuthorityType::CloseAccount,
    //         &owner2_key,
    //         &[],
    //     )
    //         .unwrap(),
    //     vec![&mut account_account, &mut owner2_account],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: None,
        authority_type: AuthorityType::CloseAccount as u8,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // wrong owner
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         set_authority(
    //             &program_id,
    //             &mint_key,
    //             Some(&owner3_key),
    //             AuthorityType::MintTokens,
    //             &owner2_key,
    //             &[]
    //         )
    //             .unwrap(),
    //         vec![&mut mint_account, &mut owner2_account],
    //     )
    // );
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &mint_key,
        new_authority: Some(&owner3_key),
        authority_type: AuthorityType::MintTokens as u8,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    // // owner did not sign
    // let mut instruction = set_authority(
    //     &program_id,
    //     &mint_key,
    //     Some(&owner2_key),
    //     AuthorityType::MintTokens,
    //     &owner_key,
    //     &[],
    // )
    //     .unwrap();
    // instruction.accounts[1].is_signer = false;
    // assert_eq!(
    //     Err(ProgramError::MissingRequiredSignature),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         instruction,
    //         vec![&mut mint_account, &mut owner_account],
    //     )
    // );
    // cannot manipulate is_signer flag

    // // cannot freeze
    // assert_eq!(
    //     Err(TokenError::MintCannotFreeze.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         set_authority(
    //             &program_id,
    //             &mint_key,
    //             Some(&owner2_key),
    //             AuthorityType::FreezeAccount,
    //             &owner_key,
    //             &[],
    //         )
    //             .unwrap(),
    //         vec![&mut mint_account, &mut owner_account],
    //     )
    // );
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &mint_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::FreezeAccount as u8,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(16)"));

    // // set owner
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &mint_key,
    //         Some(&owner2_key),
    //         AuthorityType::MintTokens,
    //         &owner_key,
    //         &[],
    //     )
    //         .unwrap(),
    //     vec![&mut mint_account, &mut owner_account],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &mint_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::MintTokens as u8,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // set owner to None
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &mint_key,
    //         None,
    //         AuthorityType::MintTokens,
    //         &owner2_key,
    //         &[],
    //     )
    //         .unwrap(),
    //     vec![&mut mint_account, &mut owner2_account],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &mint_key,
        new_authority: None,
        authority_type: AuthorityType::MintTokens as u8,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // test unsetting mint_authority is one-way operation
    // assert_eq!(
    //     Err(TokenError::FixedSupply.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         set_authority(
    //             &program_id,
    //             &mint2_key,
    //             Some(&owner2_key),
    //             AuthorityType::MintTokens,
    //             &owner_key,
    //             &[]
    //         )
    //             .unwrap(),
    //         vec![&mut mint_account, &mut owner_account],
    //     )
    // );
    // cannot manipulate specific accounts (automatically loaded)

    // // set freeze_authority
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &mint2_key,
    //         Some(&owner2_key),
    //         AuthorityType::FreezeAccount,
    //         &owner_key,
    //         &[],
    //     )
    //         .unwrap(),
    //     vec![&mut mint2_account, &mut owner_account],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &mint2_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::FreezeAccount as u8,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // test unsetting freeze_authority is one-way operation
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &mint2_key,
    //         None,
    //         AuthorityType::FreezeAccount,
    //         &owner2_key,
    //         &[],
    //     )
    //         .unwrap(),
    //     vec![&mut mint2_account, &mut owner2_account],
    // )
    //     .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &mint2_key,
        new_authority: None,
        authority_type: AuthorityType::FreezeAccount as u8,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // assert_eq!(
    //     Err(TokenError::MintCannotFreeze.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         set_authority(
    //             &program_id,
    //             &mint2_key,
    //             Some(&owner2_key),
    //             AuthorityType::FreezeAccount,
    //             &owner_key,
    //             &[],
    //         )
    //             .unwrap(),
    //         vec![&mut mint2_account, &mut owner2_account],
    //     )
    // );
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &mint2_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::FreezeAccount as u8,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(16)"));
}

#[test]
fn test_burn_abi() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let program_id = token_2022::lib::id();
    let account_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let account2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let account3_key = pubkey_from_evm_address::<true>(&USER_ADDRESS3);
    let delegate_key = pubkey_from_evm_address::<true>(&USER_ADDRESS4);
    let mismatch_key = pubkey_from_evm_address::<true>(&USER_ADDRESS5);
    let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS6);
    let owner2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS7);
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS8);
    let mint2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS9);
    let not_program_id = pubkey_from_evm_address::<true>(&USER_ADDRESS10);

    // // create new mint
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap(),
    //     vec![&mut mint_account /*&mut rent_sysvar*/],
    // )
    // .unwrap();
    let decimals = 2;
    let mut input_data = vec![];
    InitializeMintParams {
        mint: &mint_key,
        mint_authority: &owner_key,
        freeze_opt: None,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_MINT), &input_data);
    let input = build_input_raw(&ERC20_MAGIC_BYTES, &input);
    let contract_address = ctx.deploy_evm_tx(USER_ADDRESS6, input.into());

    // // create account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);

    // // create another account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account2_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account2_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account2_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);

    // // create another account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account3_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account3_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account3_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);

    // // create mismatch account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &mismatch_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut mismatch_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &mismatch_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);

    // // mint to account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     mint_to(&program_id, &mint_key, &account_key, &owner_key, &[], 1000).unwrap(),
    //     vec![&mut mint_account, &mut account_account, &mut owner_account],
    // )
    // .unwrap();
    let amount = 1000;
    let mut input_data = vec![];
    MintToParams {
        mint: &mint_key,
        account: &account_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_MINT_TO), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address).unwrap();
    assert_eq!(result, vec![1]);

    // // mint to mismatch account and change mint key
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     mint_to(&program_id, &mint_key, &mismatch_key, &owner_key, &[], 1000).unwrap(),
    //     vec![&mut mint_account, &mut mismatch_account, &mut owner_account],
    // )
    // .unwrap();
    // let mut account = Account::unpack_unchecked(&mismatch_account.data).unwrap();
    // account.mint = mint2_key;
    // Account::pack(account, &mut mismatch_account.data).unwrap();
    let amount = 1000;
    let mut input_data = vec![];
    MintToParams {
        mint: &mint_key,
        account: &mismatch_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_MINT_TO), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address).unwrap();
    assert_eq!(result, vec![1]);
    with_svm_account_mut(&mut ctx, &mismatch_key, |mismatch_account| {
        let mut account = Account::unpack_unchecked(&mismatch_account.data).unwrap();
        account.mint = mint2_key;
        Account::pack(account, &mut mismatch_account.data).unwrap();
    });

    // // missing signer
    // let mut instruction =
    //     burn(&program_id, &account_key, &mint_key, &delegate_key, &[], 42).unwrap();
    // instruction.accounts[1].is_signer = false;
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         instruction,
    //         vec![
    //             &mut account_account,
    //             &mut mint_account,
    //             &mut delegate_account
    //         ],
    //     )
    // );
    // cannot manipulate is_signer

    // // missing owner
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn(&program_id, &account_key, &mint_key, &owner2_key, &[], 42).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner2_account],
    //     )
    // );
    let amount = 42;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner2_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    // // account not owned by program
    // let not_program_id = Pubkey::new_unique();
    // account_account.owner = not_program_id;
    // assert_eq!(
    //     Err(ProgramError::IncorrectProgramId),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn(&program_id, &account_key, &mint_key, &owner_key, &[], 0).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner_account],
    //     )
    // );
    // account_account.owner = program_id;
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        account_account.owner = not_program_id;
    });
    let amount = 0;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(
        result.1,
        utf8_to_bytes("failed to process: IncorrectProgramId")
    );
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        account_account.owner = program_id;
    });

    // // mint not owned by program
    // let not_program_id = Pubkey::new_unique();
    // mint_account.owner = not_program_id;
    // assert_eq!(
    //     Err(ProgramError::IncorrectProgramId),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn(&program_id, &account_key, &mint_key, &owner_key, &[], 0).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner_account],
    //     )
    // );
    // mint_account.owner = program_id;
    with_svm_account_mut(&mut ctx, &mint_key, |mint_account| {
        mint_account.owner = not_program_id;
    });
    let amount = 0;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(
        result.1,
        utf8_to_bytes("failed to process: IncorrectProgramId")
    );
    with_svm_account_mut(&mut ctx, &mint_key, |mint_account| {
        mint_account.owner = program_id;
    });

    // // mint mismatch
    // assert_eq!(
    //     Err(TokenError::MintMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn(&program_id, &mismatch_key, &mint_key, &owner_key, &[], 42).unwrap(),
    //         vec![&mut mismatch_account, &mut mint_account, &mut owner_account],
    //     )
    // );
    let amount = 42;
    let mut input_data = vec![];
    BurnParams {
        account: &mismatch_key,
        mint: &mint_key,
        authority: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(3)"));

    // // burn
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     burn(&program_id, &account_key, &mint_key, &owner_key, &[], 21).unwrap(),
    //     vec![&mut account_account, &mut mint_account, &mut owner_account],
    // )
    // .unwrap();
    let amount = 21;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    let result = output_data.unwrap();
    assert_eq!(result, vec![1]);

    // // burn_checked, with incorrect decimals
    // assert_eq!(
    //     Err(TokenError::MintDecimalsMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn_checked(&program_id, &account_key, &mint_key, &owner_key, &[], 21, 3).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner_account],
    //     )
    // );
    let wrong_decimals = 3;
    let amount = 21;
    let mut input_data = vec![];
    BurnCheckedParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner_key,
        amount: &amount,
        decimals: wrong_decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN_CHECKED), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(18)"));

    // // burn_checked
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     burn_checked(&program_id, &account_key, &mint_key, &owner_key, &[], 21, 2).unwrap(),
    //     vec![&mut account_account, &mut mint_account, &mut owner_account],
    // )
    // .unwrap();
    let amount = 21;
    let mut input_data = vec![];
    BurnCheckedParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner_key,
        amount: &amount,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN_CHECKED), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    let result = output_data.unwrap();
    assert_eq!(result, vec![1]);

    // let mint = Mint::unpack_unchecked(&mint_account.data).unwrap();
    // assert_eq!(mint.supply, 2000 - 42);
    with_svm_account_mut(&mut ctx, &mint_key, |mint_account| {
        let mint = Mint::unpack_unchecked(&mint_account.data).unwrap();
        assert_eq!(mint.supply, 2000 - 42);
    });
    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.amount, 1000 - 42);
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.amount, 1000 - 42);
    });

    // // insufficient funds
    // assert_eq!(
    //     Err(TokenError::InsufficientFunds.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn(
    //             &program_id,
    //             &account_key,
    //             &mint_key,
    //             &owner_key,
    //             &[],
    //             100_000_000
    //         )
    //         .unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner_account],
    //     )
    // );
    let amount = 100_000_000;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(1)"));

    // // approve delegate
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     approve(
    //         &program_id,
    //         &account_key,
    //         &delegate_key,
    //         &owner_key,
    //         &[],
    //         84,
    //     )
    //     .unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut delegate_account,
    //         &mut owner_account,
    //     ],
    // )
    // .unwrap();
    let amount = 84;
    let mut input_data = vec![];
    ApproveParams {
        source: &account_key,
        delegate: &delegate_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_APPROVE), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS6, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // not a delegate of source account
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn(
    //             &program_id,
    //             &account_key,
    //             &mint_key,
    //             &owner2_key, // <-- incorrect owner or delegate
    //             &[],
    //             1,
    //         )
    //         .unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner2_account],
    //     )
    // );
    let amount = 1;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner2_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS7, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    // // insufficient funds approved via delegate
    // assert_eq!(
    //     Err(TokenError::InsufficientFunds.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn(&program_id, &account_key, &mint_key, &delegate_key, &[], 85).unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut mint_account,
    //             &mut delegate_account
    //         ],
    //     )
    // );
    let amount = 85;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &delegate_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(1)"));

    // // burn via delegate
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     burn(&program_id, &account_key, &mint_key, &delegate_key, &[], 84).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut delegate_account,
    //     ],
    // )
    // .unwrap();
    let amount = 84;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &delegate_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // match
    // let mint = Mint::unpack_unchecked(&mint_account.data).unwrap();
    // assert_eq!(mint.supply, 2000 - 42 - 84);
    with_svm_account_mut(&mut ctx, &mint_key, |mint_account| {
        let mint = Mint::unpack_unchecked(&mint_account.data).unwrap();
        assert_eq!(mint.supply, 2000 - 42 - 84);
    });
    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.amount, 1000 - 42 - 84);
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.amount, 1000 - 42 - 84);
    });

    // // insufficient funds approved via delegate
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         burn(&program_id, &account_key, &mint_key, &delegate_key, &[], 1).unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut mint_account,
    //             &mut delegate_account
    //         ],
    //     )
    // );
    let amount = 1;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &delegate_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS4, &contract_address);
    let result = output_data.unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));
}

#[test]
fn test_freeze_account_abi() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let account_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let account_owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS3);
    let owner2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS4);
    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS5);

    // // create new mint with owner different from account owner
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap(),
    //     vec![&mut mint_account /*&mut rent_sysvar*/],
    // )
    // .unwrap();
    let decimals = 2;
    let mut input_data = vec![];
    InitializeMintParams {
        mint: &mint_key,
        mint_authority: &owner_key,
        freeze_opt: None,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_MINT), &input_data);
    let input = build_input_raw(&ERC20_MAGIC_BYTES, &input);
    let contract_address = ctx.deploy_evm_tx(USER_ADDRESS6, input.into());

    // // create account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account_key, &mint_key, &account_owner_key).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut account_owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &account_owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);

    // // mint to account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     mint_to(&program_id, &mint_key, &account_key, &owner_key, &[], 1000).unwrap(),
    //     vec![&mut mint_account, &mut account_account, &mut owner_account],
    // )
    // .unwrap();
    let amount = 1000;
    let mut input_data = vec![];
    MintToParams {
        mint: &mint_key,
        account: &account_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_MINT_TO), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], 1);

    // // mint cannot freeze
    // assert_eq!(
    //     Err(TokenError::MintCannotFreeze.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         freeze_account(&program_id, &account_key, &mint_key, &owner_key, &[]).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner_account],
    //     )
    // );
    let mut input_data = vec![];
    FreezeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_FREEZE_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(16)"));

    // // missing freeze_authority
    // let mut mint = Mint::unpack_unchecked(&mint_account.data).unwrap();
    // mint.freeze_authority = COption::Some(owner_key);
    // Mint::pack(mint, &mut mint_account.data).unwrap();
    with_svm_account_mut(&mut ctx, &mint_key, |mint_account| {
        let mut mint = Mint::unpack_unchecked(&mint_account.data).unwrap();
        mint.freeze_authority = COption::Some(owner_key);
        Mint::pack(mint, &mut mint_account.data).unwrap();
    });
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         freeze_account(&program_id, &account_key, &mint_key, &owner2_key, &[]).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner2_account],
    //     )
    // );
    let mut input_data = vec![];
    FreezeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_FREEZE_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    // // check explicit thaw
    // assert_eq!(
    //     Err(TokenError::InvalidState.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         thaw_account(&program_id, &account_key, &mint_key, &owner2_key, &[]).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner2_account],
    //     )
    // );
    let mut input_data = vec![];
    ThawAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_THAW_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(13)"));

    // // freeze
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     freeze_account(&program_id, &account_key, &mint_key, &owner_key, &[]).unwrap(),
    //     vec![&mut account_account, &mut mint_account, &mut owner_account],
    // )
    // .unwrap();
    let mut input_data = vec![];
    FreezeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_FREEZE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);
    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.state, AccountState::Frozen);
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.state, AccountState::Frozen);
    });

    // // check explicit freeze
    // assert_eq!(
    //     Err(TokenError::InvalidState.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         freeze_account(&program_id, &account_key, &mint_key, &owner_key, &[]).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner_account],
    //     )
    // );
    let mut input_data = vec![];
    FreezeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_FREEZE_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(13)"));

    // // check thaw authority
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         thaw_account(&program_id, &account_key, &mint_key, &owner2_key, &[]).unwrap(),
    //         vec![&mut account_account, &mut mint_account, &mut owner2_account],
    //     )
    // );
    let mut input_data = vec![];
    ThawAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_THAW_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    // // thaw
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     thaw_account(&program_id, &account_key, &mint_key, &owner_key, &[]).unwrap(),
    //     vec![&mut account_account, &mut mint_account, &mut owner_account],
    // )
    // .unwrap();
    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.state, AccountState::Initialized);
    let mut input_data = vec![];
    ThawAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_THAW_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.state, AccountState::Initialized);
    })
}

#[test]
fn test_close_account_abi() {
    let mut ctx = EvmTestingContext::default().with_full_genesis();
    ctx.sdk = ctx.sdk.with_contract_context(ContractContextV1 {
        address: PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME,
        ..Default::default()
    });
    ctx.sdk
        .set_ownable_account_address(PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME);

    let mint_key = pubkey_from_evm_address::<true>(&USER_ADDRESS1);
    let account_key = pubkey_from_evm_address::<true>(&USER_ADDRESS2);
    let _account2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS3);
    let account3_key = pubkey_from_evm_address::<true>(&USER_ADDRESS4);
    let owner_key = pubkey_from_evm_address::<true>(&USER_ADDRESS5);
    let owner2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS6);

    // // initialize and mint to non-native account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_mint(&program_id, &mint_key, &owner_key, None, 2).unwrap(),
    //     vec![&mut mint_account /*&mut rent_sysvar*/],
    // )
    // .unwrap();
    let decimals = 2;
    let mut input_data = vec![];
    InitializeMintParams {
        mint: &mint_key,
        mint_authority: &owner_key,
        freeze_opt: None,
        decimals,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_MINT), &input_data);
    let input = build_input_raw(&ERC20_MAGIC_BYTES, &input);
    let contract_address = ctx.deploy_evm_tx(USER_ADDRESS6, input.into());

    // // uninitialized
    // assert_eq!(
    //     Err(ProgramError::UninitializedAccount),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         close_account(&program_id, &account_key, &account3_key, &owner2_key, &[]).unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut account3_account,
    //             &mut owner2_account,
    //         ],
    //     )
    // );
    let mut input_data = vec![];
    CloseAccountParams {
        account: &account_key,
        destination: &account3_key,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_CLOSE_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(
        result.1,
        utf8_to_bytes("failed to process: UninitializedAccount")
    );

    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);
    ctx.commit_db_to_sdk();

    // do_process_instruction(
    //     &mut ctx.sdk,
    //     mint_to(&program_id, &mint_key, &account_key, &owner_key, &[], 42).unwrap(),
    //     vec![
    //         &mut mint_account,
    //         &mut account_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    let amount = 42;
    let mut input_data = vec![];
    MintToParams {
        mint: &mint_key,
        account: &account_key,
        owner: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_MINT_TO), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();
    assert_eq!(result, vec![1]);

    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.amount, 42);
    ctx.commit_db_to_sdk();
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.amount, 42);
    });

    // // close non-native account with balance
    // assert_eq!(
    //     Err(TokenError::NonNativeHasBalance.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         close_account(&program_id, &account_key, &account3_key, &owner_key, &[]).unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut account3_account,
    //             &mut owner_account,
    //         ],
    //     )
    // );
    // assert_eq!(account_account.lamports, account_minimum_balance());
    let mut input_data = vec![];
    CloseAccountParams {
        account: &account_key,
        destination: &account3_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_CLOSE_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(11)"));
    ctx.commit_db_to_sdk();
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        assert_eq!(account_account.lamports, account_minimum_balance());
    });

    // // empty account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     burn(&program_id, &account_key, &mint_key, &owner_key, &[], 42).unwrap(),
    //     vec![&mut account_account, &mut mint_account, &mut owner_account],
    // )
    // .unwrap();
    let amount = 42;
    let mut input_data = vec![];
    BurnParams {
        account: &account_key,
        mint: &mint_key,
        authority: &owner_key,
        amount: &amount,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_BURN), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);

    // // wrong owner
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         close_account(&program_id, &account_key, &account3_key, &owner2_key, &[]).unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut account3_account,
    //             &mut owner2_account,
    //         ],
    //     )
    // );
    let mut input_data = vec![];
    CloseAccountParams {
        account: &account_key,
        destination: &account3_key,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_CLOSE_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    // // close account
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     close_account(&program_id, &account_key, &account3_key, &owner_key, &[]).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut account3_account,
    //         &mut owner_account,
    //     ],
    // )
    // .unwrap();
    // assert_eq!(account3_account.lamports, 2 * account_minimum_balance());
    // assert_eq!(account_account.lamports, 0);
    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.amount, 0);
    let mut input_data = vec![];
    CloseAccountParams {
        account: &account_key,
        destination: &account3_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_CLOSE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap();
    assert_eq!(result, vec![1]);
    ctx.commit_db_to_sdk();
    with_svm_account_mut(&mut ctx, &account3_key, |account3_account| {
        assert_eq!(account3_account.lamports, 2 * account_minimum_balance());
    });
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        assert_eq!(account_account.lamports, 0);
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.amount, 0);
    });

    // // fund and initialize new non-native account to test close authority
    // let account_key = Pubkey::new_unique();
    let account_key = pubkey_from_evm_address::<true>(&USER_ADDRESS7);
    // let mut account_account = SolanaAccount::new(
    //     account_minimum_balance(),
    //     Account::get_packed_len(),
    //     &program_id,
    // );
    // let owner2_key = Pubkey::new_unique();
    let owner2_key = pubkey_from_evm_address::<true>(&USER_ADDRESS8);
    // let mut owner2_account = SolanaAccount::new(
    //     account_minimum_balance(),
    //     Account::get_packed_len(),
    //     &program_id,
    // );
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     initialize_account(&program_id, &account_key, &mint_key, &owner_key).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut mint_account,
    //         &mut owner_account,
    //         // &mut rent_sysvar,
    //     ],
    // )
    // .unwrap();
    // account_account.lamports = 2;
    let mut input_data = vec![];
    InitializeAccountParams {
        account: &account_key,
        mint: &mint_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_INITIALIZE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS3, &contract_address).unwrap();
    assert_eq!(result, vec![1]);
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        account_account.lamports = 2;
    });

    // do_process_instruction(
    //     &mut ctx.sdk,
    //     set_authority(
    //         &program_id,
    //         &account_key,
    //         Some(&owner2_key),
    //         AuthorityType::CloseAccount,
    //         &owner_key,
    //         &[],
    //     )
    //     .unwrap(),
    //     vec![&mut account_account, &mut owner_account],
    // )
    // .unwrap();
    let mut input_data = vec![];
    SetAuthorityParams {
        owned: &account_key,
        new_authority: Some(&owner2_key),
        authority_type: AuthorityType::CloseAccount as u8,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_SET_AUTHORITY), &input_data);
    let output_data = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address);
    assert_eq!(output_data.unwrap(), vec![1]);
    ctx.commit_db_to_sdk();
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        assert_eq!(account_account.lamports, 2);
    });

    // // account owner cannot authorize close if close_authority is set
    // assert_eq!(
    //     Err(TokenError::OwnerMismatch.into()),
    //     do_process_instruction(
    //         &mut ctx.sdk,
    //         close_account(&program_id, &account_key, &account3_key, &owner_key, &[]).unwrap(),
    //         vec![
    //             &mut account_account,
    //             &mut account3_account,
    //             &mut owner_account,
    //         ],
    //     )
    // );
    let mut input_data = vec![];
    CloseAccountParams {
        account: &account_key,
        destination: &account3_key,
        owner: &owner_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_CLOSE_ACCOUNT), &input_data);
    let result =
        call_with_sig(&mut ctx, input.into(), &USER_ADDRESS5, &contract_address).unwrap_err();
    assert_eq!(result.0, u32::MAX);
    assert_eq!(result.1, utf8_to_bytes("failed to process: Custom(4)"));

    // // close non-native account with close_authority
    // do_process_instruction(
    //     &mut ctx.sdk,
    //     close_account(&program_id, &account_key, &account3_key, &owner2_key, &[]).unwrap(),
    //     vec![
    //         &mut account_account,
    //         &mut account3_account,
    //         &mut owner2_account,
    //     ],
    // )
    // .unwrap();
    // assert_eq!(account3_account.lamports, 2 * account_minimum_balance() + 2);
    // assert_eq!(account_account.lamports, 0);
    // let account = Account::unpack_unchecked(&account_account.data).unwrap();
    // assert_eq!(account.amount, 0);
    let mut input_data = vec![];
    CloseAccountParams {
        account: &account_key,
        destination: &account3_key,
        owner: &owner2_key,
    }
    .serialize_into(&mut input_data);
    let input = build_input_raw(&sig_to_bytes(SIG_CLOSE_ACCOUNT), &input_data);
    let result = call_with_sig(&mut ctx, input.into(), &USER_ADDRESS8, &contract_address).unwrap();
    assert_eq!(result, vec![1]);
    ctx.commit_db_to_sdk();
    with_svm_account_mut(&mut ctx, &account_key, |account_account| {
        assert_eq!(account_account.lamports, 0);
        let account = Account::unpack_unchecked(&account_account.data).unwrap();
        assert_eq!(account.amount, 0);
    });
    with_svm_account_mut(&mut ctx, &account3_key, |account3_account| {
        assert_eq!(account3_account.lamports, 2 * account_minimum_balance() + 2);
    });
}
