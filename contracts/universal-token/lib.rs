#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

use alloc::vec::Vec;
use fluentbase_sdk::{
    debug_log_ext, entrypoint, Address, ContextReader, SharedAPI, ERC20_MAGIC_BYTES,
};
use fluentbase_svm::fluentbase::token2022::{token2022_process, token2022_process_raw};
use fluentbase_svm::pubkey::{Pubkey, PUBKEY_BYTES};
use fluentbase_svm::token_2022;
use fluentbase_svm::token_2022::extension::ExtensionType;
use fluentbase_svm::token_2022::instruction::AuthorityType;
use fluentbase_svm::token_2022::processor::Processor;
use fluentbase_svm_common::common::{
    lamports_to_bytes, pubkey_from_evm_address, pubkey_try_from_slice,
};
use fluentbase_svm_common::universal_token::{
    ApproveCheckedParams, ApproveParams, BurnCheckedParams, BurnParams, CloseAccountParams,
    FreezeAccountParams, GetAccountDataSizeParams, InitializeAccountParams, InitializeMintParams,
    MintToParams, RevokeParams, SetAuthorityParams, ThawAccountParams, TransferFromParams,
    TransferParams,
};
use fluentbase_universal_token::{
    common::bytes_to_sig,
    consts::{
        ERR_MALFORMED_INPUT, SIG_APPROVE, SIG_APPROVE_CHECKED, SIG_BALANCE, SIG_BALANCE_OF,
        SIG_BURN, SIG_BURN_CHECKED, SIG_CLOSE_ACCOUNT, SIG_DECIMALS, SIG_FREEZE_ACCOUNT,
        SIG_GET_ACCOUNT_DATA_SIZE, SIG_INITIALIZE_ACCOUNT, SIG_INITIALIZE_MINT, SIG_MINT_TO,
        SIG_REVOKE, SIG_SET_AUTHORITY, SIG_THAW_ACCOUNT, SIG_TOKEN2022, SIG_TRANSFER,
        SIG_TRANSFER_FROM,
    },
    storage::SIG_LEN_BYTES,
};

fn decimals_for_pubkey<SDK: SharedAPI>(sdk: &mut SDK, pk: &Pubkey) -> u8 {
    let mut processor = Processor::new(sdk);
    let decimals = processor.decimals(&pk).expect("failed to get decimals");
    decimals
}

fn decimals<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(pk) = pubkey_try_from_slice(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let decimals = decimals_for_pubkey(sdk, &pk);
    sdk.write(&[decimals]);
}

fn transfer<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let from = &pubkey_from_evm_address::<true>(&sdk.context().contract_caller());
    let Ok(params) = TransferParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::transfer_checked(
        &token_2022::lib::id(),
        &from,
        &params.mint,
        &params.to,
        &params.authority,
        &[],
        params.amount,
        params.decimals,
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
    let Ok(params) = TransferFromParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    #[allow(deprecated)]
    let instruction = token_2022::instruction::transfer_checked(
        &token_2022::lib::id(),
        &params.from,
        &params.mint,
        &params.to,
        &params.authority,
        &[],
        *params.amount,
        params.decimals,
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
    debug_log_ext!("IS_DEPLOY={}", IS_DEPLOY);
    let Ok(params) = InitializeMintParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::initialize_mint(
        &token_2022::lib::id(),
        &params.mint,
        &params.mint_authority,
        params.freeze_opt,
        params.decimals,
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
    let Ok(params) = InitializeAccountParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::initialize_account(
        &token_2022::lib::id(),
        &params.account,
        &params.mint,
        &params.owner,
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
    let Ok(params) = MintToParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::mint_to(
        &token_2022::lib::id(),
        &params.mint,
        &params.account,
        &params.owner,
        &[],
        *params.amount,
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
    let Ok(params) = ApproveParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::approve(
        &token_2022::lib::id(),
        &params.from,
        &params.delegate,
        &params.owner,
        &[],
        *params.amount,
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
    let Ok(params) = ApproveCheckedParams::try_parse(input) else {
        debug_log_ext!();
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::approve_checked(
        &token_2022::lib::id(),
        &params.source,
        &params.mint,
        &params.delegate,
        &params.owner,
        &[],
        *params.amount,
        params.decimals,
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
    let Ok(params) = RevokeParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction =
        token_2022::instruction::revoke(&token_2022::lib::id(), &params.source, &params.owner, &[])
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

fn set_authority<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let Ok(params) = SetAuthorityParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::set_authority(
        &token_2022::lib::id(),
        &params.owned,
        params.new_authority,
        AuthorityType::from(params.authority_type).expect("invalid AuthorityType"),
        &params.owner,
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
    let Ok(params) = BurnParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::burn(
        &token_2022::lib::id(),
        &params.account,
        &params.mint,
        &params.authority,
        &[],
        *params.amount,
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
    let Ok(params) = BurnCheckedParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::burn_checked(
        &token_2022::lib::id(),
        params.account,
        params.mint,
        params.authority,
        &[],
        *params.amount,
        params.decimals,
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
    let Ok(params) = CloseAccountParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::close_account(
        &token_2022::lib::id(),
        params.account,
        params.destination,
        params.owner,
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
    let Ok(params) = FreezeAccountParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::freeze_account(
        &token_2022::lib::id(),
        params.account,
        params.mint,
        params.owner,
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
    let Ok(params) = ThawAccountParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let instruction = token_2022::instruction::thaw_account(
        &token_2022::lib::id(),
        &params.account,
        &params.mint,
        &params.owner,
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
    let Ok(params) = GetAccountDataSizeParams::try_parse(input) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let extension_types: Result<Vec<ExtensionType>, _> = params
        .extension_types
        .iter()
        .map(|v| ExtensionType::try_from(*v))
        .collect();
    let extension_types = extension_types.expect("valid extension types");
    let instruction = token_2022::instruction::get_account_data_size(
        &token_2022::lib::id(),
        params.mint,
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
    if sig1_bytes != ERC20_MAGIC_BYTES {
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
        SIG_BALANCE => balance(&mut sdk),              //
        SIG_BALANCE_OF => balance_of(&mut sdk, input), //
        SIG_INITIALIZE_MINT => initialize_mint::<_, false>(&mut sdk, input), //
        SIG_TRANSFER => transfer(&mut sdk, input),     //
        SIG_TRANSFER_FROM => transfer_from(&mut sdk, input), //
        SIG_INITIALIZE_ACCOUNT => initialize_account(&mut sdk, input), //
        SIG_MINT_TO => mint_to(&mut sdk, input),       //
        SIG_DECIMALS => decimals(&mut sdk, input),     //
        SIG_APPROVE => approve(&mut sdk, input),       //
        SIG_APPROVE_CHECKED => approve_checked(&mut sdk, input), //
        SIG_REVOKE => revoke(&mut sdk, input),         //
        SIG_SET_AUTHORITY => set_authority(&mut sdk, input), //
        SIG_BURN => burn(&mut sdk, input),             //
        SIG_BURN_CHECKED => burn_checked(&mut sdk, input), //
        SIG_FREEZE_ACCOUNT => freeze_account(&mut sdk, input), //
        SIG_THAW_ACCOUNT => thaw_account(&mut sdk, input), //
        SIG_CLOSE_ACCOUNT => close_account(&mut sdk, input),
        SIG_GET_ACCOUNT_DATA_SIZE => get_account_data_size(&mut sdk, input),
        // SIG_TOTAL_SUPPLY => total_supply(&mut sdk),
        SIG_TOKEN2022 => {
            token2022_process_raw::<false, _>(&mut sdk, input).expect("failed to process")
        }
        _ => {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        }
    }
}

entrypoint!(main_entry, deploy_entry);
