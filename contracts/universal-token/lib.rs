#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

use core::array::TryFromSliceError;
use fluentbase_sdk::{debug_log_ext, entrypoint, Address, ContextReader, SharedAPI};
use fluentbase_svm::fluentbase::token2022::{token2022_process, token2022_process_raw};
use fluentbase_svm::pubkey::{Pubkey, PUBKEY_BYTES};
use fluentbase_svm::token_2022;
use fluentbase_svm::token_2022::processor::Processor;
use fluentbase_svm_common::common::pubkey_from_evm_address;
use fluentbase_universal_token::{
    common::{bytes_to_sig, u256_from_bytes_slice_try},
    consts::{
        ERR_ALREADY_PAUSED, ERR_ALREADY_UNPAUSED, ERR_INVALID_PAUSER, ERR_MALFORMED_INPUT,
        ERR_PAUSABLE_PLUGIN_NOT_ACTIVE, SIG_BALANCE, SIG_BALANCE_OF, SIG_TOKEN2022,
        SIG_TRANSFER_FROM,
    },
    storage::{Config, Settings, ADDRESS_LEN_BYTES, SIG_LEN_BYTES, U256_LEN_BYTES},
};

fn balance_try_from_slice(input: &[u8]) -> Result<u64, TryFromSliceError> {
    let amount_bytes: [u8; size_of::<u64>()] = input[..size_of::<u64>()].try_into()?;
    Ok(u64::from_be_bytes(amount_bytes))
}

fn balance_to_bytes(balance: u64) -> [u8; size_of::<u64>()] {
    balance.to_be_bytes()
}

fn pubkey_try_from_slice(input: &[u8]) -> Result<Pubkey, TryFromSliceError> {
    let bytes: [u8; PUBKEY_BYTES] = input[..PUBKEY_BYTES].try_into()?;
    Ok(Pubkey::new_from_array(bytes))
}

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
    let from = sdk.context().contract_caller();
    const FROM_OFFSET: usize = 0;
    const MINTER_OFFSET: usize = FROM_OFFSET + PUBKEY_BYTES;
    const TO_OFFSET: usize = MINTER_OFFSET + PUBKEY_BYTES;
    const AUTHORITY_OFFSET: usize = TO_OFFSET + PUBKEY_BYTES;
    const AMOUNT_OFFSET: usize = AUTHORITY_OFFSET + PUBKEY_BYTES;

    let Ok(from) = pubkey_try_from_slice(&input[FROM_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(minter) = pubkey_try_from_slice(&input[MINTER_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(to) = pubkey_try_from_slice(&input[TO_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(authority) = pubkey_try_from_slice(&input[AUTHORITY_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(amount) = balance_try_from_slice(&input[AMOUNT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    let program_id = fluentbase_svm::token_2022::lib::id();
    let instruction = token_2022::instruction::transfer_checked(
        &program_id,
        &from,
        &minter,
        &to,
        &authority,
        &[],
        amount,
        2, // TODO put as params in input?
    )
    .unwrap();

    let mut processor = Processor::new(sdk);
    processor
        .process_extended::<false>(
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
    const MINTER_OFFSET: usize = FROM_OFFSET + PUBKEY_BYTES;
    const TO_OFFSET: usize = MINTER_OFFSET + PUBKEY_BYTES;
    const AUTHORITY_OFFSET: usize = TO_OFFSET + PUBKEY_BYTES;
    const AMOUNT_OFFSET: usize = AUTHORITY_OFFSET + PUBKEY_BYTES;

    let Ok(amount) = balance_try_from_slice(&input[AMOUNT_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(from) = pubkey_try_from_slice(&input[FROM_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(minter) = pubkey_try_from_slice(&input[MINTER_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(to) = pubkey_try_from_slice(&input[TO_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(authority) = pubkey_try_from_slice(&input[AUTHORITY_OFFSET..]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };

    debug_log_ext!("amount {}", amount);

    let program_id = token_2022::lib::id();
    let instruction = token_2022::instruction::transfer_checked(
        &program_id,
        &from,
        &minter,
        &to,
        &authority,
        &[],
        amount,
        2, // TODO put as params in input?
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
    let Ok(amount) = balance_try_from_slice(&input[AMOUNT_OFFSET..]) else {
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
    sdk.write(&balance_to_bytes(balance))
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
    let (_sig, input) = sdk.input().split_at(SIG_LEN_BYTES);
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
        // SIG_TRANSFER => transfer(&mut sdk, input),
        SIG_TRANSFER_FROM => transfer_from(&mut sdk, input),
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
