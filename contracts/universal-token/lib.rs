#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

use alloc::vec::Vec;
use fluentbase_sdk::bincode::Encode;
use fluentbase_sdk::{
    system_entrypoint2, Address, Bytes, ContextReader, ExitCode, RuntimeInterruptionOutcomeV1,
    RuntimeNewFrameInputV1, RuntimeUniversalTokenOutputV1, SharedAPI, U256,
};
use fluentbase_universal_token::consts::{ERR_INSUFFICIENT_BALANCE, ERR_UNKNOWN};
use fluentbase_universal_token::helpers::bincode::{decode, encode};
use fluentbase_universal_token::services::storage_global::{
    get_slot_key_at, prepare_query_batch, print_stats, storage_service,
};
use fluentbase_universal_token::storage::{
    allowance_service, balance_service, init_services, settings_service,
};
use fluentbase_universal_token::types::result_or_interruption::ResultOrInterruption;
use fluentbase_universal_token::{
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
    storage::{Config, Feature, InitialSettings, ADDRESS_LEN_BYTES, SIG_LEN_BYTES, U256_LEN_BYTES},
    unwrap,
};

macro_rules! return_custom_err {
    ($e:ident) => {
        return Err(($e.to_le_bytes().into(), ExitCode::Panic));
    };
}

macro_rules! return_interruption {
    ($params:expr) => {
        return Err(($params, ExitCode::InterruptionCalled));
    };
    () => {
        return_interruption!(Bytes::new())
    };
}

fn symbol(_input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let symbol: Bytes = unwrap!(settings_service(false).symbol()).into();
    symbol.into()
}
fn name(_input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let name: Bytes = unwrap!(settings_service(false).name()).into();
    name.into()
}
fn decimals(_input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let output: Bytes =
        fixed_bytes_from_u256(&unwrap!(settings_service(false).decimals_get())).into();
    output.into()
}

fn transfer(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let from = sdk.context().contract_caller();
    const TO_OFFSET: usize = 0;
    const AMOUNT_OFFSET: usize = TO_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(to) = Address::try_from(&input[TO_OFFSET..TO_OFFSET + ADDRESS_LEN_BYTES]) else {
        return ERR_MALFORMED_INPUT.into();
    };
    let Some(amount) =
        u256_from_bytes_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_LEN_BYTES])
    else {
        return ERR_MALFORMED_INPUT.into();
    };
    if !unwrap!(balance_service(false).send(&from, &to, &amount)) {
        return ERR_INSUFFICIENT_BALANCE.into();
    };
    print_stats();
    emit_transfer_event(sdk, &from, &to, &amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn transfer_from(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    const FROM_OFFSET: usize = 0;
    const TO_OFFSET: usize = FROM_OFFSET + ADDRESS_LEN_BYTES;
    const AMOUNT_OFFSET: usize = TO_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(to) = Address::try_from(&input[TO_OFFSET..TO_OFFSET + ADDRESS_LEN_BYTES]) else {
        return ERR_MALFORMED_INPUT.into();
    };
    let Some(amount) =
        u256_from_bytes_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + U256_LEN_BYTES])
    else {
        return ERR_MALFORMED_INPUT.into();
    };
    let Ok(from) = Address::try_from(&input[FROM_OFFSET..FROM_OFFSET + ADDRESS_LEN_BYTES]) else {
        return ERR_MALFORMED_INPUT.into();
    };
    if !unwrap!(allowance_service(false).subtract(&from, &to, &amount)) {
        return ERR_INSUFFICIENT_ALLOWANCE.into();
    }
    if !unwrap!(balance_service(false).send(&from, &to, &amount)) {
        return ERR_INSUFFICIENT_BALANCE.into();
    };
    emit_transfer_event(sdk, &from, &to, &amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn approve(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
    const AMOUNT_OFFSET: usize = SPENDER_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        return ERR_MALFORMED_INPUT.into();
    };
    let Ok(spender) = Address::try_from(&input[SPENDER_OFFSET..SPENDER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        return ERR_MALFORMED_INPUT.into();
    };
    let Some(amount) =
        u256_from_bytes_slice_try(&input[AMOUNT_OFFSET..AMOUNT_OFFSET + size_of::<U256>()])
    else {
        return ERR_MALFORMED_INPUT.into();
    };
    allowance_service(false).update(&owner, &spender, &amount);
    emit_approval_event(sdk, &owner, &spender, &amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn allow(input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    const OWNER_OFFSET: usize = 0;
    const SPENDER_OFFSET: usize = OWNER_OFFSET + ADDRESS_LEN_BYTES;
    let Ok(owner) = Address::try_from(&input[OWNER_OFFSET..OWNER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        return ERR_MALFORMED_INPUT.into();
    };
    let Ok(spender) = Address::try_from(&input[SPENDER_OFFSET..SPENDER_OFFSET + ADDRESS_LEN_BYTES])
    else {
        return ERR_MALFORMED_INPUT.into();
    };
    let amount = unwrap!(allowance_service(false).get(&owner, &spender));
    let bytes: Bytes = fixed_bytes_from_u256(&amount).into();
    bytes.into()
}

fn total_supply(_input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let result = unwrap!(settings_service(false).total_supply_get());
    let result: Bytes = fixed_bytes_from_u256(&result).into();
    result.into()
}

fn balance_of(input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let Ok(owner) = Address::try_from(&input[..ADDRESS_LEN_BYTES]) else {
        return ERR_MALFORMED_INPUT.into();
    };
    let result = unwrap!(balance_service(false).get(&owner));
    let result: Bytes = fixed_bytes_from_u256(&result).into();
    result.into()
}

fn mint(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let mut config = Config::new(false);
    if !unwrap!(config.mintable_plugin_enabled()) {
        return ERR_MINTABLE_PLUGIN_NOT_ACTIVE.into();
    }
    let minter = sdk.context().contract_caller();
    if minter != unwrap!(settings_service(false).minter_get()) {
        return ERR_INVALID_MINTER.into();
    }
    if unwrap!(config.pausable_plugin_enabled()) && unwrap!(config.paused()) {
        return ERR_PAUSABLE_PLUGIN_NOT_ACTIVE.into();
    }
    let Ok(to) = Address::try_from(&input[..ADDRESS_LEN_BYTES]) else {
        return ERR_MALFORMED_INPUT.into();
    };
    let zero_address = Address::ZERO;
    if to == zero_address {
        return ERR_INVALID_RECIPIENT.into();
    }
    let Some(amount) =
        u256_from_bytes_slice_try(&input[ADDRESS_LEN_BYTES..ADDRESS_LEN_BYTES + U256_LEN_BYTES])
    else {
        return ERR_MALFORMED_INPUT.into();
    };
    let total_supply: U256 = unwrap!(settings_service(false).total_supply_get());
    let (total_supply, overflow) = total_supply.overflowing_add(amount);
    if overflow {
        return ERR_OVERFLOW.into();
    }
    settings_service(false).total_supply_set(&total_supply);
    unwrap!(balance_service(false).add(&to, &amount));
    emit_transfer_event(sdk, &zero_address, &to, &amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn pause(sdk: &mut impl SharedAPI, _input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let mut config = Config::new(false);
    if !unwrap!(config.pausable_plugin_enabled().map_err(|e| ERR_UNKNOWN)) {
        return ERR_PAUSABLE_PLUGIN_NOT_ACTIVE.into();
    }
    let pauser = sdk.context().contract_caller();
    let current_pauser: Address = unwrap!(settings_service(false)
        .pauser_get()
        .map_err(|e| ERR_UNKNOWN));
    if pauser != current_pauser {
        return ERR_INVALID_PAUSER.into();
    }
    if unwrap!(config.paused().map_err(|e| ERR_UNKNOWN)) {
        return ERR_ALREADY_PAUSED.into();
    }
    config.pause();
    config.save_flags();
    emit_pause_event(sdk, &pauser);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn unpause(sdk: &mut impl SharedAPI, _input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    let mut config = Config::new(false);
    if !unwrap!(config.pausable_plugin_enabled()) {
        return ERR_PAUSABLE_PLUGIN_NOT_ACTIVE.into();
    }
    let pauser = sdk.context().contract_caller();
    let current_pauser: Address = unwrap!(settings_service(false).pauser_get());
    if pauser != current_pauser {
        return ERR_INVALID_PAUSER.into();
    }
    if !unwrap!(config.paused()) {
        return ERR_ALREADY_UNPAUSED.into();
    }
    config.unpause();
    config.save_flags();
    emit_unpause_event(sdk, &pauser);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn try_process_read_query_batch<const READ: bool, const DEFAULT_ON_READ: bool>(
    sdk: &mut impl SharedAPI,
) -> bool {
    let query_batch_ptr = prepare_query_batch::<READ, DEFAULT_ON_READ>();
    if let Some(params) = query_batch_ptr {
        let output = encode(&params).unwrap();
        sdk.write(&ExitCode::InterruptionCalled.into_i32().to_le_bytes());
        sdk.write(&output);
        return true;
    }
    false
}

#[inline(never)]
pub fn deploy_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<Bytes, (Bytes, ExitCode)> {
    init_services(true);

    let input = sdk.bytes_input();
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        return_custom_err!(ERR_MALFORMED_INPUT)
    }
    let (new_frame_input, _) = decode::<RuntimeNewFrameInputV1>(&input).unwrap();
    let (_sig, input) = new_frame_input.input.split_at(SIG_LEN_BYTES);
    let initial_settings = InitialSettings::try_decode_from_slice(&input);
    let (initial_settings, _) = if let Ok(v) = initial_settings {
        v
    } else {
        return_custom_err!(ERR_DECODE);
    };
    if !initial_settings.is_valid() {
        return_custom_err!(ERR_VALIDATION);
    }
    let mut config = Config::new(true);
    for feature in initial_settings.features() {
        let result: ResultOrInterruption<(), u32> = match feature {
            Feature::Meta { name, symbol } => {
                if !settings_service(true).name_set(name) {
                    return_custom_err!(ERR_INVALID_META_NAME);
                }
                if !settings_service(true).symbol_set(symbol) {
                    return_custom_err!(ERR_INVALID_META_SYMBOL);
                }
                ().into()
            }
            Feature::InitialSupply {
                amount,
                owner,
                decimals,
            } => {
                let amount = u256_from_fixed_bytes(amount);
                let owner = owner.into();
                settings_service(true).decimals_set(*decimals);
                settings_service(true).total_supply_set(&amount);
                balance_service(true).add(&owner, &amount)
            }
            Feature::Mintable { minter } => {
                config.enable_mintable_plugin();
                settings_service(true).minter_set(&Address::from(minter));
                ().into()
            }
            Feature::Pausable { pauser } => {
                config.enable_pausable_plugin();
                settings_service(true).pauser_set(&Address::from(pauser));
                ().into()
            }
        };
        match result {
            ResultOrInterruption::Result(r) => match r {
                Ok(_) => {}
                Err(e) => {
                    return_custom_err!(e);
                }
            },
            ResultOrInterruption::Interruption() => {
                unreachable!();
            }
        }
    }
    config.save_flags();
    // TODO process accumulated result if presented
    print_stats();
    let query_batch_ptr = prepare_query_batch::<false, false>();
    if let Some(params) = query_batch_ptr {
        let output = encode(&params).unwrap();
        sdk.write(&ExitCode::InterruptionCalled.into_i32().to_le_bytes());
        sdk.write(&output);
    } else {
        sdk.write(&ExitCode::Ok.into_i32().to_le_bytes());
        let mut storage =
            Vec::<([u8; 32], [u8; 32])>::with_capacity(storage_service(true).values_new().len());
        for v in storage_service(true).values_new() {
            storage.push((v.0.to_le_bytes(), v.1.to_le_bytes()))
        }
        let output = encode(&RuntimeUniversalTokenOutputV1 {
            output: Vec::new(),
            storage,
        })
        .unwrap();
        sdk.write(&output);
        storage_service(true).clear();
    }
    Ok(Bytes::new())
}

#[inline(never)]
pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<Bytes, (Bytes, ExitCode)> {
    print_stats();
    init_services(false);

    let return_data = sdk.return_data();
    if !return_data.is_empty() {
        let (out, _) = decode::<RuntimeInterruptionOutcomeV1>(&return_data).unwrap();
        assert_eq!(out.output.len(), 32);
        let slot = get_slot_key_at(0);
        let value = U256::from_le_slice(&out.output);
        storage_service(false).set_existing(&slot, &value);
        if try_process_read_query_batch::<true, false>(sdk) {
            return_interruption!()
        };
    }

    let input = sdk.input();
    let (new_frame_input, _) = decode::<RuntimeNewFrameInputV1>(input).unwrap();

    let input_size = new_frame_input.input.len() as u32;
    if input_size < SIG_LEN_BYTES as u32 {
        return_custom_err!(ERR_MALFORMED_INPUT);
    }
    let (sig, input) = new_frame_input.input.split_at(SIG_LEN_BYTES);
    let signature = bytes_to_sig(sig);
    let result: ResultOrInterruption<Bytes, u32> = match signature {
        SIG_SYMBOL => symbol(input),
        SIG_NAME => name(input),
        SIG_TRANSFER => transfer(sdk, input),
        SIG_TRANSFER_FROM => transfer_from(sdk, input),
        SIG_APPROVE => approve(sdk, input),
        SIG_DECIMALS => decimals(input),
        SIG_ALLOWANCE => allow(input),
        SIG_TOTAL_SUPPLY => total_supply(input),
        SIG_BALANCE_OF => balance_of(input),
        SIG_MINT => mint(sdk, input),
        SIG_PAUSE => pause(sdk, input),
        SIG_UNPAUSE => unpause(sdk, input),
        _ => {
            return_custom_err!(ERR_MALFORMED_INPUT)
        }
    };
    match result {
        ResultOrInterruption::Result(r) => match r {
            Ok(v) => {
                print_stats();
                let output = encode(&RuntimeUniversalTokenOutputV1 {
                    output: v.into(),
                    storage: {
                        let mut s = storage_service(false);
                        let result = s
                            .values_new()
                            .iter()
                            .map(|(k, v)| (k.to_le_bytes(), v.to_le_bytes()))
                            .collect();
                        s.clear();
                        result
                    },
                })
                .unwrap();
                return Ok(output.into());
            }
            Err(e) => {
                storage_service(false).clear();
                return_custom_err!(e)
            }
        },
        ResultOrInterruption::Interruption() => {
            print_stats();
            if try_process_read_query_batch::<true, false>(sdk) {
                return_interruption!()
            };
        }
    }
    print_stats();
    Ok(Bytes::new())
}

system_entrypoint2!(main_entry, deploy_entry);
