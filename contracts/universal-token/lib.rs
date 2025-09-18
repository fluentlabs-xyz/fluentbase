#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

use fluentbase_sdk::{
    debug_log_ext, entrypoint, Address, ContextReader, SharedAPI, ERC20_MAGIC_BYTES,
};
use fluentbase_svm::fluentbase::token2022::{token2022_process, token2022_process_raw};
use fluentbase_svm::pubkey::{Pubkey, PUBKEY_BYTES};
use fluentbase_svm::token_2022;
use fluentbase_svm::token_2022::processor::Processor;
use fluentbase_svm_common::common::{
    lamports_to_bytes, lamports_try_from_slice, pubkey_from_evm_address, pubkey_try_from_slice,
};
use fluentbase_universal_token::{
    common::{bytes_to_sig, u256_from_bytes_slice_try},
    consts::{
        ERR_ALREADY_PAUSED, ERR_ALREADY_UNPAUSED, ERR_INVALID_PAUSER, ERR_MALFORMED_INPUT,
        ERR_PAUSABLE_PLUGIN_NOT_ACTIVE, SIG_BALANCE, SIG_BALANCE_OF, SIG_INITIALIZE_ACCOUNT,
        SIG_INITIALIZE_MINT, SIG_MINT_TO, SIG_TOKEN2022, SIG_TRANSFER, SIG_TRANSFER_FROM,
    },
    storage::{Config, Settings, ADDRESS_LEN_BYTES, SIG_LEN_BYTES, U256_LEN_BYTES},
};

fn symbol<SDK: SharedAPI>(sdk: &mut SDK) {
    // sdk.write(&get_symbol(sdk));
}
fn name<SDK: SharedAPI>(sdk: &mut SDK) {
    // sdk.write(&get_name(sdk));
}
fn decimals<SDK: SharedAPI>(sdk: &mut SDK) {
    // let output = fixed_bytes_from_u256(&get_decimals(sdk));
    sdk.write(&[0]);
}

fn transfer<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let from = &pubkey_from_evm_address::<true>(&sdk.context().contract_caller());
    const TO_OFFSET: usize = 0;
    const AUTHORITY_OFFSET: usize = TO_OFFSET + PUBKEY_BYTES;
    const AMOUNT_OFFSET: usize = AUTHORITY_OFFSET + PUBKEY_BYTES;

    let Ok(amount) = lamports_try_from_slice(&input[AMOUNT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(to) = pubkey_try_from_slice(&input[TO_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(authority) = pubkey_try_from_slice(&input[AUTHORITY_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    #[allow(deprecated)]
    let instruction = token_2022::instruction::transfer(
        &token_2022::lib::id(),
        &from,
        &to,
        &authority,
        &[],
        amount,
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
    // let from = sdk.context().contract_caller();
    const FROM_OFFSET: usize = 0;
    const TO_OFFSET: usize = FROM_OFFSET + PUBKEY_BYTES;
    const AUTHORITY_OFFSET: usize = TO_OFFSET + PUBKEY_BYTES;
    const AMOUNT_OFFSET: usize = AUTHORITY_OFFSET + PUBKEY_BYTES;

    let Ok(amount) = lamports_try_from_slice(&input[AMOUNT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(from) = pubkey_try_from_slice(&input[FROM_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(to) = pubkey_try_from_slice(&input[TO_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(authority) = pubkey_try_from_slice(&input[AUTHORITY_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    #[allow(deprecated)]
    let instruction = token_2022::instruction::transfer(
        &token_2022::lib::id(),
        &from,
        &to,
        &authority,
        &[],
        amount,
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

fn initialize_mint<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    const MINT_OFFSET: usize = 0;
    const MINT_AUTHORITY_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
    const FREEZE_OFFSET: usize = MINT_AUTHORITY_OFFSET + PUBKEY_BYTES;
    const DECIMALS_OFFSET: usize = FREEZE_OFFSET + PUBKEY_BYTES;

    let Some(decimals) = input.get(DECIMALS_OFFSET).cloned() else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(mint) = pubkey_try_from_slice(&input[MINT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(mint_authority) = pubkey_try_from_slice(&input[MINT_AUTHORITY_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(freeze) = pubkey_try_from_slice(&input[FREEZE_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let freeze_opt = if freeze == Pubkey::default() {
        None
    } else {
        Some(freeze)
    };

    let instruction = token_2022::instruction::initialize_mint(
        &token_2022::lib::id(),
        &mint,
        &mint_authority,
        freeze_opt.as_ref(),
        decimals,
    )
    .unwrap();

    token2022_process::<true, _>(
        sdk,
        &instruction.program_id,
        &instruction.accounts,
        &instruction.data,
    )
    .expect("failed to process");

    // sdk.write(&[1]);
}

fn initialize_account<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    const ACCOUNT_OFFSET: usize = 0;
    const MINT_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
    const OWNER_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;

    let Ok(owner) = pubkey_try_from_slice(&input[OWNER_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(account) = pubkey_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(mint) = pubkey_try_from_slice(&input[MINT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::initialize_account(
        &token_2022::lib::id(),
        &account,
        &mint,
        &owner,
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
    const MINT_OFFSET: usize = 0;
    const ACCOUNT_OFFSET: usize = MINT_OFFSET + PUBKEY_BYTES;
    const OWNER_OFFSET: usize = ACCOUNT_OFFSET + PUBKEY_BYTES;
    const AMOUNT_OFFSET: usize = OWNER_OFFSET + PUBKEY_BYTES;

    let Ok(amount) = lamports_try_from_slice(&input[AMOUNT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(mint) = pubkey_try_from_slice(&input[MINT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(account) = pubkey_try_from_slice(&input[ACCOUNT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(owner) = pubkey_try_from_slice(&input[OWNER_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let instruction = token_2022::instruction::mint_to(
        &token_2022::lib::id(),
        &mint,
        &account,
        &owner,
        &[],
        amount,
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
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
    const AMOUNT_OFFSET: usize = SPENDER_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(spender) = Address::try_from(&input[SPENDER_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(amount) = lamports_try_from_slice(&input[AMOUNT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    // do_approve(&mut sdk, &owner, &spender, &amount);
    sdk.write(&[0]);
}

fn allow<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(spender) = Address::try_from(&input[SPENDER_OFFSET..SPENDER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    // let amount = get_allowance(&mut sdk, &owner, &spender);
    sdk.write(&[0]);
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

fn mint<SDK: SharedAPI>(sdk: &mut SDK, input: &[u8]) {
    let minter = sdk.context().contract_caller();
    let Ok(to) = Address::try_from(&input[..ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        u256_from_bytes_slice_try(&input[ADDRESS_LEN_BYTES..ADDRESS_LEN_BYTES + U256_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let mut config = Config::new();
    // evm_exit!(sdk, do_mint(&mut sdk, &mut config, &minter, &to, &amount));
    sdk.write(&[0])
}

fn pause<SDK: SharedAPI>(sdk: &mut SDK) {
    let mut config = Config::new();
    if !config.pausable_plugin_enabled(sdk) {
        sdk.evm_exit(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let pauser = sdk.context().contract_caller();
    if pauser != Settings::pauser_get(sdk) {
        sdk.evm_exit(ERR_INVALID_PAUSER);
    }
    if config.paused(sdk) {
        sdk.evm_exit(ERR_ALREADY_PAUSED);
    }
    // evm_exit!(sdk, do_pause(sdk, &mut config, &pauser));
    sdk.write(&[1]);
}

fn unpause<SDK: SharedAPI>(sdk: &mut SDK, _input: &[u8]) {
    let mut config = Config::new();
    if !config.pausable_plugin_enabled(sdk) {
        sdk.evm_exit(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let pauser = sdk.context().contract_caller();
    if pauser != Settings::pauser_get(sdk) {
        sdk.evm_exit(ERR_INVALID_PAUSER);
    }
    if !config.paused(sdk) {
        sdk.evm_exit(ERR_ALREADY_UNPAUSED);
    }
    // evm_exit!(sdk, do_unpause(sdk, &mut config, &pauser));
    sdk.write(&[0]);
}

pub fn deploy_entry(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let (sig1_bytes, input) = sdk.input().split_at(SIG_LEN_BYTES);
    if sig1_bytes != ERC20_MAGIC_BYTES {
        panic!("invalid input signature");
    }
    let sig2_bytes = &input[..SIG_LEN_BYTES];
    let sig2 = bytes_to_sig(sig2_bytes);
    debug_log_ext!("sig1_bytes {:x?} sig2_bytes {:x?}", sig1_bytes, sig2_bytes);
    match sig2 {
        SIG_INITIALIZE_MINT => {
            // TODO check for collision or add specific signature for raw processing
            debug_log_ext!("initialize_mint");
            initialize_mint(&mut sdk, &input[SIG_LEN_BYTES..]);
            return;
        }
        _ => {}
    }
    token2022_process_raw::<true, _>(&mut sdk, input).expect("failed to process token deploy");
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
        // SIG_SYMBOL => symbol(sdk),
        // SIG_NAME => name(sdk),
        SIG_BALANCE => balance(&mut sdk),
        SIG_BALANCE_OF => balance_of(&mut sdk, input),
        SIG_TRANSFER => transfer(&mut sdk, input),
        SIG_TRANSFER_FROM => transfer_from(&mut sdk, input),
        SIG_INITIALIZE_ACCOUNT => initialize_account(&mut sdk, input),
        SIG_MINT_TO => mint_to(&mut sdk, input),
        // SIG_APPROVE => approve(&mut sdk, input),
        // SIG_DECIMALS => decimals(&mut sdk),
        // SIG_ALLOWANCE => allow(&mut sdk, input),
        // SIG_TOTAL_SUPPLY => total_supply(&mut sdk),
        // SIG_MINT => mint(&mut sdk, input),
        // SIG_PAUSE => pause(&mut sdk),
        // SIG_UNPAUSE => unpause(&mut sdk, input),
        SIG_TOKEN2022 => {
            token2022_process_raw::<false, _>(&mut sdk, input).expect("failed to process")
        }
        _ => {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        }
    }
}

entrypoint!(main_entry, deploy_entry);
