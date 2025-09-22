#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

use alloc::vec::Vec;
use fluentbase_erc20::{
    common::{
        bytes_to_sig, fixed_bytes_from_u256, u256_from_bytes_slice_try, u256_from_fixed_bytes,
    },
    consts::{
        emit_approval_event, emit_pause_event, emit_transfer_event, emit_unpause_event,
        ERR_ALREADY_PAUSED, ERR_ALREADY_UNPAUSED, ERR_DECODE, ERR_INSUFFICIENT_ALLOWANCE,
        ERR_INVALID_META_NAME, ERR_INVALID_META_SYMBOL, ERR_INVALID_MINTER, ERR_INVALID_PAUSER,
        ERR_INVALID_RECIPIENT, ERR_MALFORMED_INPUT, ERR_MINTABLE_PLUGIN_NOT_ACTIVE, ERR_OVERFLOW,
        ERR_PAUSABLE_PLUGIN_NOT_ACTIVE, ERR_VALIDATION, SIG_ALLOWANCE, SIG_APPROVE, SIG_BALANCE_OF,
        SIG_DECIMALS, SIG_MINT, SIG_NAME, SIG_PAUSE, SIG_SYMBOL, SIG_TOTAL_SUPPLY, SIG_TRANSFER,
        SIG_TRANSFER_FROM, SIG_UNPAUSE,
    },
    storage::{
        Allowance, Balance, Config, Feature, InitialSettings, Settings, ADDRESS_LEN_BYTES,
        SIG_LEN_BYTES, U256_LEN_BYTES,
    },
};
use fluentbase_sdk::{debug_log_ext, entrypoint, Address, ContextReader, SharedAPI, U256};
use fluentbase_svm::{
    pubkey::{Pubkey, PUBKEY_BYTES},
    solana_program::instruction::AccountMeta,
};
use solana_program_error::ProgramResult;

fn symbol(mut sdk: impl SharedAPI, _input: &[u8]) {
    sdk.write(&Settings::symbol(&sdk));
}
fn name(mut sdk: impl SharedAPI, _input: &[u8]) {
    sdk.write(&Settings::name(&sdk));
}
fn decimals(mut sdk: impl SharedAPI, _input: &[u8]) {
    let output = fixed_bytes_from_u256(&Settings::decimals_get(&sdk));
    sdk.write(&output);
}

fn transfer(mut sdk: impl SharedAPI, input: &[u8]) {
    let from = sdk.context().contract_caller();
    const TO_OFFSET: usize = 0;
    const AMOUNT_OFFSET: usize = TO_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(to) = Address::try_from(&input[TO_OFFSET..TO_OFFSET + ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        u256_from_bytes_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    Balance::send(&mut sdk, from, to, amount);
    emit_transfer_event(&mut sdk, &from, &to, &amount);
    let result = fixed_bytes_from_u256(&U256::from(1));
    sdk.write(&result);
}

fn transfer_from(mut sdk: impl SharedAPI, input: &[u8]) {
    let spender = sdk.context().contract_caller();
    const FROM_OFFSET: usize = 0;
    const TO_OFFSET: usize = FROM_OFFSET + ADDRESS_LEN_BYTES;
    const AMOUNT_OFFSET: usize = TO_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(to) = Address::try_from(&input[TO_OFFSET..TO_OFFSET + ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        u256_from_bytes_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let from = {
        let Ok(from) = Address::try_from(&input[FROM_OFFSET..FROM_OFFSET + ADDRESS_LEN_BYTES])
        else {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        };
        if !Allowance::subtract(&mut sdk, from, spender, amount) {
            sdk.evm_exit(ERR_INSUFFICIENT_ALLOWANCE);
        }
        from
    };
    Balance::send(&mut sdk, from, to, amount);
    emit_transfer_event(&mut sdk, &from, &to, &amount);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)));
}

fn approve(mut sdk: impl SharedAPI, input: &[u8]) {
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
    const AMOUNT_OFFSET: usize = SPENDER_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Ok(spender) = Address::try_from(&input[SPENDER_OFFSET..SPENDER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let Some(amount) =
        u256_from_bytes_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + size_of::<U256>()])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    Allowance::update(&mut sdk, owner, spender, amount);
    emit_approval_event(&mut sdk, &owner, &spender, &amount);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)));
}

fn allow(mut sdk: impl SharedAPI, input: &[u8]) {
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
    let amount = Allowance::get_current(&mut sdk, owner, spender);
    sdk.write(&fixed_bytes_from_u256(&amount));
}

fn total_supply(mut sdk: impl SharedAPI, _input: &[u8]) {
    let result = Settings::total_supply_get(&sdk);
    sdk.write(&fixed_bytes_from_u256(&result))
}

fn balance_of(mut sdk: impl SharedAPI, input: &[u8]) {
    let Ok(owner) = Address::try_from(&input[..ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let result = Balance::get_for(&sdk, owner);
    sdk.write(&fixed_bytes_from_u256(&result))
}

fn mint(mut sdk: impl SharedAPI, input: &[u8]) {
    let mut config = Config::new();
    if !config.mintable_plugin_enabled(&mut sdk) {
        sdk.evm_exit(ERR_MINTABLE_PLUGIN_NOT_ACTIVE);
    }
    let minter = sdk.context().contract_caller();
    if minter != Settings::minter_get(&sdk) {
        sdk.evm_exit(ERR_INVALID_MINTER);
    }
    if config.pausable_plugin_enabled(&mut sdk) && config.paused(&mut sdk) {
        sdk.evm_exit(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let Ok(to) = Address::try_from(&input[..ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let zero_address = Address::ZERO;
    if to == zero_address {
        sdk.evm_exit(ERR_INVALID_RECIPIENT);
    }
    let Some(amount) =
        u256_from_bytes_slice_try(&input[ADDRESS_LEN_BYTES..ADDRESS_LEN_BYTES + U256_LEN_BYTES])
    else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let total_supply = Settings::total_supply_get(&sdk);
    let (total_supply, overflow) = total_supply.overflowing_add(amount);
    if overflow {
        sdk.evm_exit(ERR_OVERFLOW);
    }
    Settings::total_supply_set(&mut sdk, total_supply);
    Balance::add(&mut sdk, to, amount);
    emit_transfer_event(&mut sdk, &zero_address, &to, &amount);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)))
}

fn pause(mut sdk: impl SharedAPI, _input: &[u8]) {
    let mut config = Config::new();
    if !config.pausable_plugin_enabled(&mut sdk) {
        sdk.evm_exit(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let pauser = sdk.context().contract_caller();
    if pauser != Settings::pauser_get(&sdk) {
        sdk.evm_exit(ERR_INVALID_PAUSER);
    }
    if config.paused(&mut sdk) {
        sdk.evm_exit(ERR_ALREADY_PAUSED);
    }
    config.pause(&mut sdk);
    config.save_flags(&mut sdk);
    emit_pause_event(&mut sdk, &pauser);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)));
}

fn unpause(mut sdk: impl SharedAPI, _input: &[u8]) {
    let mut config = Config::new();
    if !config.pausable_plugin_enabled(&mut sdk) {
        sdk.evm_exit(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let pauser = sdk.context().contract_caller();
    if pauser != Settings::pauser_get(&sdk) {
        sdk.evm_exit(ERR_INVALID_PAUSER);
    }
    if !config.paused(&mut sdk) {
        sdk.evm_exit(ERR_ALREADY_UNPAUSED);
    }
    config.unpause(&mut sdk);
    config.save_flags(&mut sdk);
    emit_unpause_event(&mut sdk, &pauser);
    sdk.write(&fixed_bytes_from_u256(&U256::from(1)));
}

fn token2022(_sdk: impl SharedAPI, input: &[u8]) -> ProgramResult {
    debug_log_ext!("token2022(): input.len={} input={:x?}", input.len(), input);
    // input: program_id (pk 32 bytes) + accounts_meta_number (u8) + account_meta[] (AccountMeta) + data ([u8])
    let mut offset = 0;
    let program_id =
        Pubkey::new_from_array(input[offset..offset + PUBKEY_BYTES].try_into().unwrap());
    offset += PUBKEY_BYTES;
    let account_meta_count = input[offset] as usize;
    let mut account_metas = Vec::with_capacity(account_meta_count);
    offset += 1;
    for i in 0..account_meta_count {
        let account_meta: AccountMeta =
            solana_bincode::deserialize(&input[offset..offset + size_of::<AccountMeta>()])
                .expect("failed to deserialize AccountMeta");
        // TODO extract data for account info
        account_metas.push(account_meta);
        offset += size_of::<AccountMeta>();
    }
    offset += size_of::<AccountMeta>() * account_meta_count;
    let _input = &input[offset..];
    // Processor::process(&program_id, &account_metas, input)
    // sdk.write(&fixed_bytes_from_u256(&U256::from(1)));
    ProgramResult::Ok(())
}

pub fn deploy_entry(mut sdk: impl SharedAPI) {
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let (_sig, input) = sdk.input().split_at(SIG_LEN_BYTES);
    let initial_settings = InitialSettings::try_decode_from_slice(&input);
    let (initial_settings, _) = if let Ok(v) = initial_settings {
        v
    } else {
        sdk.evm_exit(ERR_DECODE);
    };
    if !initial_settings.is_valid() {
        sdk.evm_exit(ERR_VALIDATION);
    }
    let mut config = Config::new();
    for feature in initial_settings.features() {
        match feature {
            Feature::Meta { name, symbol } => {
                if !Settings::name_set(&mut sdk, name) {
                    sdk.evm_exit(ERR_INVALID_META_NAME);
                }
                if !Settings::symbol_set(&mut sdk, symbol) {
                    sdk.evm_exit(ERR_INVALID_META_SYMBOL);
                }
            }
            Feature::InitialSupply {
                amount,
                owner,
                decimals,
            } => {
                let amount = u256_from_fixed_bytes(&mut sdk, amount);
                let owner = owner.into();
                Settings::decimals_set(&mut sdk, U256::from(*decimals));
                Settings::total_supply_set(&mut sdk, amount);
                Balance::add(&mut sdk, owner, amount);
            }
            Feature::Mintable { minter } => {
                config.enable_mintable_plugin(&mut sdk);
                Settings::minter_set(&mut sdk, &Address::from(minter));
            }
            Feature::Pausable { pauser } => {
                config.enable_pausable_plugin(&mut sdk);
                Settings::pauser_set(&mut sdk, &Address::from(pauser));
            }
        }
    }
    config.save_flags(&mut sdk);
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
        SIG_SYMBOL => symbol(sdk, input),
        SIG_NAME => name(sdk, input),
        SIG_TRANSFER => transfer(sdk, input),
        SIG_TRANSFER_FROM => transfer_from(sdk, input),
        SIG_APPROVE => approve(sdk, input),
        SIG_DECIMALS => decimals(sdk, input),
        SIG_ALLOWANCE => allow(sdk, input),
        SIG_TOTAL_SUPPLY => total_supply(sdk, input),
        SIG_BALANCE_OF => balance_of(sdk, input),
        SIG_MINT => mint(sdk, input),
        SIG_PAUSE => pause(sdk, input),
        SIG_UNPAUSE => unpause(sdk, input),
        // SIG_TOKEN2022 => token2022(sdk, input).expect("failed to process token2022 input"),
        _ => {
            sdk.evm_exit(ERR_MALFORMED_INPUT);
        }
    }
}

entrypoint!(main_entry, deploy_entry);
