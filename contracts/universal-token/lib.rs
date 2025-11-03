#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

use alloc::vec::Vec;
use fluentbase_sdk::{
    debug_log_ext, entrypoint, Address, ContextReader, SharedAPI, UNIVERSAL_TOKEN_MAGIC_BYTES,
};
use fluentbase_svm::{
    fluentbase::token2022::{token2022_process, token2022_process_raw},
    pubkey::{Pubkey, PUBKEY_BYTES},
    token_2022,
    token_2022::{extension::ExtensionType, instruction::AuthorityType, processor::Processor},
};
use fluentbase_svm_common::{
    common::{lamports_to_bytes, pubkey_from_evm_address, pubkey_try_from_slice},
    universal_token::{
        AllowanceParams, ApproveCheckedParams, ApproveParams, BurnCheckedParams, BurnParams,
        CloseAccountParams, FreezeAccountParams, GetAccountDataSizeParams, InitializeAccountParams,
        InitializeMintParams, MintToParams, RevokeParams, SetAuthorityParams, ThawAccountParams,
        TransferFromParams, TransferParams,
    },
};
use fluentbase_universal_token::{
    common::bytes_to_sig,
    consts::{
        ERR_MALFORMED_INPUT, SIG_ALLOWANCE, SIG_APPROVE, SIG_APPROVE_CHECKED, SIG_BALANCE,
        SIG_BALANCE_OF, SIG_BURN, SIG_BURN_CHECKED, SIG_CLOSE_ACCOUNT, SIG_DECIMALS,
        SIG_DECIMALS_FOR_MINT, SIG_FREEZE_ACCOUNT, SIG_GET_ACCOUNT_DATA_SIZE,
        SIG_INITIALIZE_ACCOUNT, SIG_INITIALIZE_MINT, SIG_MINT_TO, SIG_REVOKE, SIG_SET_AUTHORITY,
        SIG_THAW_ACCOUNT, SIG_TOKEN2022, SIG_TRANSFER, SIG_TRANSFER_FROM,
    },
    storage::SIG_LEN_BYTES,
};
use solana_program_error::ProgramError;

fn decimals_for_mint_pubkey<SDK: SharedAPI>(
    sdk: &mut SDK,
    pk: &Pubkey,
) -> Result<u8, ProgramError> {
    let mut processor = Processor::new(sdk);
    let decimals = processor.decimals_for_mint(&pk)?;
    Ok(decimals)
}

fn decimals_for_account_pubkey<SDK: SharedAPI>(
    sdk: &mut SDK,
    pk: &Pubkey,
) -> Result<u8, ProgramError> {
    let mut processor = Processor::new(sdk);
    let decimals = processor.decimals_for_account(&pk)?;
    Ok(decimals)
}

fn decimals_for_mint<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(pk) = pubkey_try_from_slice(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let decimals = decimals_for_mint_pubkey(sdk, &pk).expect("failed to get decimals");
    sdk.write(&[decimals]);
}

fn decimals_for_account<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(pk) = pubkey_try_from_slice(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let decimals = decimals_for_account_pubkey(sdk, &pk).expect("failed to get decimals");
    sdk.write(&[decimals]);
}

fn allowance_for<SDK: SharedAPI>(
    sdk: &mut SDK,
    delegate: &Pubkey,
    account: &Pubkey,
) -> Result<u64, ProgramError> {
    let mut processor = Processor::new(sdk);
    let allowance = processor.allowance(delegate, account)?;
    Ok(allowance)
}

fn allowance<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = AllowanceParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(allowance) = allowance_for(sdk, p.delegate, p.source) else {
        sdk.write(&lamports_to_bytes(0));
        return;
    };
    sdk.write(&lamports_to_bytes(allowance));
}

fn transfer<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let from = &pubkey_from_evm_address::<true>(&sdk.context().contract_caller());
    let Ok(p) = TransferParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::transfer_checked(
        &token_2022::lib::id(),
        &from,
        &p.mint,
        &p.to,
        &p.authority,
        &[],
        p.amount,
        p.decimals,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn transfer_from<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = TransferFromParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    #[allow(deprecated)]
    let instruction = token_2022::instruction::transfer_checked(
        &token_2022::lib::id(),
        &p.from,
        &p.mint,
        &p.to,
        &p.authority,
        &[],
        *p.amount,
        p.decimals,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn initialize_mint<SDK: SharedAPI, const IS_DEPLOY: bool>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = InitializeMintParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::initialize_mint(
        &token_2022::lib::id(),
        &p.mint,
        &p.mint_authority,
        p.freeze_opt,
        p.decimals,
    )
    .unwrap();

    token2022_process::<IS_DEPLOY, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");

    if !IS_DEPLOY {
        sdk.write(&[1]);
    }
}

fn initialize_account<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = InitializeAccountParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::initialize_account(
        &token_2022::lib::id(),
        &p.account,
        &p.mint,
        &p.owner,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");

    sdk.write(&[1]);
}

fn mint_to<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = MintToParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::mint_to(
        &token_2022::lib::id(),
        &p.mint,
        &p.account,
        &p.owner,
        &[],
        *p.amount,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");

    sdk.write(&[1]);
}

fn approve<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = ApproveParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::approve(
        &token_2022::lib::id(),
        &p.source,
        &p.delegate,
        &p.owner,
        &[],
        *p.amount,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn approve_checked<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = ApproveCheckedParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::approve_checked(
        &token_2022::lib::id(),
        &p.source,
        &p.mint,
        &p.delegate,
        &p.owner,
        &[],
        *p.amount,
        p.decimals,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn revoke<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = RevokeParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction =
        token_2022::instruction::revoke(&token_2022::lib::id(), &p.source, &p.owner, &[]).unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn set_authority<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = SetAuthorityParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::set_authority(
        &token_2022::lib::id(),
        &p.owned,
        p.new_authority,
        AuthorityType::from(p.authority_type).expect("invalid AuthorityType"),
        &p.owner,
        &[],
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn burn<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = BurnParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::burn(
        &token_2022::lib::id(),
        &p.account,
        &p.mint,
        &p.authority,
        &[],
        *p.amount,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn burn_checked<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = BurnCheckedParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::burn_checked(
        &token_2022::lib::id(),
        p.account,
        p.mint,
        p.authority,
        &[],
        *p.amount,
        p.decimals,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn close_account<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = CloseAccountParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::close_account(
        &token_2022::lib::id(),
        p.account,
        p.destination,
        p.owner,
        &[],
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn freeze_account<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = FreezeAccountParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::freeze_account(
        &token_2022::lib::id(),
        p.account,
        p.mint,
        p.owner,
        &[],
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn thaw_account<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = ThawAccountParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::thaw_account(
        &token_2022::lib::id(),
        &p.account,
        &p.mint,
        &p.owner,
        &[],
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn get_account_data_size<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(p) = GetAccountDataSizeParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let extension_types: Result<Vec<ExtensionType>, _> = p
        .extension_types
        .iter()
        .map(|v| ExtensionType::try_from(*v))
        .collect();
    let extension_types = extension_types.expect("valid extension types");
    let instruction = token_2022::instruction::get_account_data_size(
        &token_2022::lib::id(),
        p.mint,
        &extension_types,
    )
    .unwrap();

    token2022_process::<false, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");
    sdk.write(&[1]);
}

fn total_supply<SDK: SharedAPI>(sdk: &mut SDK) {
    // let result = get_total_supply(sdk);
    sdk.write(&[0])
}

fn balance_for_pubkey<SDK: SharedAPI>(sdk: &mut SDK, pubkey: &Pubkey) {
    let mut processor = Processor::new(sdk);
    let balance = processor
        .balance_of(&pubkey)
        .expect("failed to get balance");
    sdk.write(&lamports_to_bytes(balance))
}

fn balance_for_address<SDK: SharedAPI>(sdk: &mut SDK, address: &Address) {
    let pubkey = pubkey_from_evm_address::<true>(&address);
    balance_for_pubkey(sdk, &pubkey);
}

fn balance<SDK: SharedAPI>(sdk: &mut SDK) {
    let contract_caller = sdk.context().contract_caller();
    balance_for_address(sdk, &contract_caller);
}

fn balance_of<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(pubkey) = Pubkey::try_from(&input[..PUBKEY_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    balance_for_pubkey(sdk, &pubkey);
}

pub fn deploy_entry(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let (sig1_bytes, input1) = sdk.input().split_at(SIG_LEN_BYTES);
    if sig1_bytes != UNIVERSAL_TOKEN_MAGIC_BYTES {
        panic!("invalid input signature");
    }
    let (sig2_bytes, input2) = &input1.split_at(SIG_LEN_BYTES);
    let sig2 = bytes_to_sig(sig2_bytes);
    match sig2 {
        SIG_INITIALIZE_MINT => {
            initialize_mint::<_, true>(&mut sdk, &input2);
            return;
        }
        _ => {}
    }
    token2022_process_raw::<true, _>(&mut sdk, input1).expect("failed to process token deploy");
}

pub fn main_entry(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    let input = sdk.input();
    if input_size < SIG_LEN_BYTES as u32 {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let (sig, input) = input.split_at(SIG_LEN_BYTES);
    let signature = bytes_to_sig(sig);
    match signature {
        SIG_BALANCE => balance(&mut sdk),
        SIG_BALANCE_OF => balance_of(&mut sdk, input),
        SIG_INITIALIZE_MINT => initialize_mint::<_, false>(&mut sdk, input),
        SIG_TRANSFER => transfer(&mut sdk, input),
        SIG_TRANSFER_FROM => transfer_from(&mut sdk, input),
        SIG_INITIALIZE_ACCOUNT => initialize_account(&mut sdk, input),
        SIG_MINT_TO => mint_to(&mut sdk, input),
        SIG_DECIMALS_FOR_MINT => decimals_for_mint(&mut sdk, input),
        SIG_DECIMALS => decimals_for_account(&mut sdk, input),
        SIG_ALLOWANCE => allowance(&mut sdk, input),
        SIG_APPROVE => approve(&mut sdk, input),
        SIG_APPROVE_CHECKED => approve_checked(&mut sdk, input),
        SIG_REVOKE => revoke(&mut sdk, input),
        SIG_SET_AUTHORITY => set_authority(&mut sdk, input),
        SIG_BURN => burn(&mut sdk, input),
        SIG_BURN_CHECKED => burn_checked(&mut sdk, input),
        SIG_FREEZE_ACCOUNT => freeze_account(&mut sdk, input),
        SIG_THAW_ACCOUNT => thaw_account(&mut sdk, input),
        SIG_CLOSE_ACCOUNT => close_account(&mut sdk, input),
        SIG_GET_ACCOUNT_DATA_SIZE => get_account_data_size(&mut sdk, input),
        SIG_TOKEN2022 => {
            token2022_process_raw::<false, _>(&mut sdk, input).expect("failed to process")
        }
        _ => {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        }
    }
}

entrypoint!(main_entry, deploy_entry);
