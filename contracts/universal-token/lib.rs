#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

use alloc::vec::Vec;
use fluentbase_sdk::bincode::Encode;
use fluentbase_sdk::syscall::SYSCALL_ID_STORAGE_READ;
use fluentbase_sdk::{
    bincode, debug_log, entrypoint, system_entrypoint, system_entrypoint2, Address, Bytes,
    ContextReader, ExitCode, RuntimeInterruptionOutcomeV1, RuntimeNewFrameInputV1,
    RuntimeUniversalTokenDeployOutputV1, RuntimeUniversalTokenInterruption,
    RuntimeUniversalTokenInterruptionV1, RuntimeUniversalTokenStorageReadBatchInterruptionV1,
    SharedAPI, SyscallInvocationParams, STATE_DEPLOY, STATE_MAIN, U256,
};
use fluentbase_universal_token::consts::{ERR_INSUFFICIENT_BALANCE, ERR_UNKNOWN};
use fluentbase_universal_token::helpers::bincode::{decode, encode};
use fluentbase_universal_token::services::storage_global::{
    get_slot_key_at, prepare_query_batch, print_stats, storage_service,
};
use fluentbase_universal_token::storage::{
    allowance_service, balance_service, init_services, settings_service,
};
use fluentbase_universal_token::types::result_or_interruption::ResultOrInt;
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

fn symbol(_input: &[u8]) -> ResultOrInt<Bytes> {
    let symbol: Bytes = unwrap!(settings_service(false).symbol()).into();
    symbol.into()
}
fn name(_input: &[u8]) -> ResultOrInt<Bytes> {
    let name: Bytes = unwrap!(settings_service(false).name()).into();
    name.into()
}
fn decimals(_input: &[u8]) -> ResultOrInt<Bytes> {
    let output: Bytes =
        fixed_bytes_from_u256(&unwrap!(settings_service(false).decimals_get())).into();
    output.into()
}

fn transfer(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInt<Bytes> {
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
    if !unwrap!(balance_service(false).send(&from, &to, &amount)) {
        sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
    };
    emit_transfer_event(sdk, &from, &to, &amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn transfer_from(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInt<Bytes> {
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
        if !unwrap!(allowance_service(false).subtract(&from, &spender, &amount)) {
            sdk.evm_exit(ERR_INSUFFICIENT_ALLOWANCE);
        }
        from
    };
    if !unwrap!(balance_service(false).send(&from, &to, &amount)) {
        sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
    };
    emit_transfer_event(sdk, &from, &to, &amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn approve(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInt<Bytes> {
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
    allowance_service(false).update(&owner, &spender, &amount);
    emit_approval_event(sdk, &owner, &spender, &amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn allow(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInt<Bytes> {
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
    let amount = unwrap!(allowance_service(false).get_current(&owner, &spender));
    let result: Bytes = fixed_bytes_from_u256(&amount).into();
    result.into()
}

fn total_supply(_input: &[u8]) -> ResultOrInt<Bytes> {
    let result = unwrap!(settings_service(false).total_supply_get());
    debug_log!("result {}", result);
    let result: Bytes = fixed_bytes_from_u256(&result).into();
    result.into()
}

fn balance_of(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInt<Bytes> {
    let Ok(owner) = Address::try_from(&input[..ADDRESS_LEN_BYTES]) else {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    };
    let result = unwrap!(balance_service(false).get(&owner));
    let result: Bytes = fixed_bytes_from_u256(&result).into();
    result.into()
}

fn mint(sdk: &mut impl SharedAPI, input: &[u8]) -> ResultOrInt<Bytes> {
    let mut config = Config::new(false);
    if !unwrap!(config.mintable_plugin_enabled()) {
        sdk.evm_exit(ERR_MINTABLE_PLUGIN_NOT_ACTIVE);
    }
    let minter = sdk.context().contract_caller();
    if minter != unwrap!(settings_service(false).minter_get()) {
        sdk.evm_exit(ERR_INVALID_MINTER);
    }
    if unwrap!(config.pausable_plugin_enabled()) && unwrap!(config.paused()) {
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
    let total_supply: U256 = unwrap!(settings_service(false).total_supply_get());
    let (total_supply, overflow) = total_supply.overflowing_add(amount);
    if overflow {
        sdk.evm_exit(ERR_OVERFLOW);
    }
    settings_service(false).total_supply_set(&total_supply);
    balance_service(false).add(&to, &amount);
    emit_transfer_event(sdk, &zero_address, &to, &amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn pause(sdk: &mut impl SharedAPI, _input: &[u8]) -> ResultOrInterruption<Bytes, u32> {
    debug_log!();
    let mut config = Config::new(false);
    if !unwrap!(config.pausable_plugin_enabled().map_err(|e| ERR_UNKNOWN)) {
        debug_log!();
        return ERR_PAUSABLE_PLUGIN_NOT_ACTIVE.into();
    }
    debug_log!();
    let pauser = sdk.context().contract_caller();
    debug_log!();
    let current_pauser: Address = unwrap!(settings_service(false)
        .pauser_get()
        .map_err(|e| ERR_UNKNOWN));
    debug_log!();
    if pauser != current_pauser {
        sdk.evm_exit(ERR_INVALID_PAUSER);
    }
    debug_log!();
    if unwrap!(config.paused().map_err(|e| ERR_UNKNOWN)) {
        sdk.evm_exit(ERR_ALREADY_PAUSED);
    }
    config.pause();
    config.save_flags();
    emit_pause_event(sdk, &pauser);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    result.into()
}

fn unpause(sdk: &mut impl SharedAPI, _input: &[u8]) -> ResultOrInt<Bytes> {
    let mut config = Config::new(false);
    if !unwrap!(config.pausable_plugin_enabled()) {
        sdk.evm_exit(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let pauser = sdk.context().contract_caller();
    let current_pauser: Address = unwrap!(settings_service(false).pauser_get());
    if pauser != current_pauser {
        sdk.evm_exit(ERR_INVALID_PAUSER);
    }
    if !unwrap!(config.paused()) {
        sdk.evm_exit(ERR_ALREADY_UNPAUSED);
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
    debug_log!("try_process_read_query_batch");
    let query_batch_ptr = prepare_query_batch::<READ, DEFAULT_ON_READ>();
    if let Some(params) = query_batch_ptr {
        let output = encode(&params).unwrap();
        debug_log!("params {:?}", params);
        sdk.write(&ExitCode::InterruptionCalled.into_i32().to_le_bytes());
        sdk.write(&output);
        return true;
    }
    false
}

#[inline(never)]
pub fn deploy_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<Bytes, (Bytes, ExitCode)> {
    debug_log!();
    init_services(true);

    let input = sdk.bytes_input();
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    debug_log!("input.len={}", input_size);
    let (new_frame_input, _) = decode::<RuntimeNewFrameInputV1>(&input).unwrap();
    debug_log!();
    let (_sig, input) = new_frame_input.input.split_at(SIG_LEN_BYTES);
    let initial_settings = InitialSettings::try_decode_from_slice(&input);
    let (initial_settings, _) = if let Ok(v) = initial_settings {
        v
    } else {
        sdk.evm_exit(ERR_DECODE);
    };
    if !initial_settings.is_valid() {
        sdk.evm_exit(ERR_VALIDATION);
    }
    let mut config = Config::new(true);
    for feature in initial_settings.features() {
        let result: ResultOrInt<()> = match feature {
            Feature::Meta { name, symbol } => {
                if !settings_service(true).name_set(name) {
                    sdk.evm_exit(ERR_INVALID_META_NAME);
                }
                if !settings_service(true).symbol_set(symbol) {
                    sdk.evm_exit(ERR_INVALID_META_SYMBOL);
                }
                ().into()
            }
            Feature::InitialSupply {
                amount,
                owner,
                decimals,
            } => {
                let amount = u256_from_fixed_bytes(amount);
                debug_log!("amount {}", amount);
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
            ResultOrInt::Result(r) => match r {
                Ok(_) => {}
                Err(_) => {
                    debug_log!("error");
                    panic!("failed to deploy: unknown error")
                }
            },
            ResultOrInt::Interruption() => {
                // TODO process int
                debug_log!("not allowed in deploy");
                panic!("int not allowed in deploy");
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
        let output = encode(&RuntimeUniversalTokenDeployOutputV1 { storage }).unwrap();
        sdk.write(&output);
        storage_service(true).clear();
    }
    debug_log!();
    Ok(Bytes::new())
}

#[inline(never)]
pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<Bytes, (Bytes, ExitCode)> {
    debug_log!(
        "storage_service(false).default_on_read={} sdk.context().contract_address()={}",
        storage_service(false).default_on_read(),
        sdk.context().contract_address(),
    );
    debug_log!();
    print_stats();
    init_services(false);

    let return_data = sdk.return_data();
    if !return_data.is_empty() {
        debug_log!();
        let (out, _) = decode::<RuntimeInterruptionOutcomeV1>(&return_data).unwrap();
        debug_log!("out.output.len={}", out.output.len());
        assert_eq!(out.output.len(), 32);
        debug_log!();
        let slot = get_slot_key_at(0);
        let value = U256::from_le_slice(&out.output);
        storage_service(false).set_existing(&slot, &value);
        debug_log!("slot {} value {}", slot, value);
        if try_process_read_query_batch::<true, false>(sdk) {
            debug_log!();
            return Err((Bytes::new(), ExitCode::InterruptionCalled));
        };
        debug_log!();
    }

    let input = sdk.input();
    let (new_frame_input, _) = decode::<RuntimeNewFrameInputV1>(input).unwrap();

    let input_size = new_frame_input.input.len() as u32;
    if input_size < SIG_LEN_BYTES as u32 {
        sdk.evm_exit(ERR_MALFORMED_INPUT);
    }
    let (sig, input) = new_frame_input.input.split_at(SIG_LEN_BYTES);
    let signature = bytes_to_sig(sig);
    debug_log!();
    let result: ResultOrInt<Bytes> = match signature {
        SIG_SYMBOL => symbol(input),
        SIG_NAME => name(input),
        SIG_TRANSFER => transfer(sdk, input),
        SIG_TRANSFER_FROM => transfer_from(sdk, input),
        SIG_APPROVE => approve(sdk, input),
        SIG_DECIMALS => decimals(input),
        SIG_ALLOWANCE => allow(sdk, input),
        SIG_TOTAL_SUPPLY => total_supply(input),
        SIG_BALANCE_OF => balance_of(sdk, input),
        SIG_MINT => mint(sdk, input),
        SIG_PAUSE => {
            let result = pause(sdk, input);
            match result {
                ResultOrInterruption::Result(r) => match r {
                    Ok(v) => v.into(),
                    Err(e) => {
                        debug_log!("error {}", e);
                        // sdk.write(&e.to_le_bytes());
                        return Err((e.to_le_bytes().into(), ExitCode::PrecompileError));
                    }
                },
                ResultOrInterruption::Interruption() => ResultOrInterruption::Interruption(),
            }
        }
        SIG_UNPAUSE => unpause(sdk, input),
        _ => {
            debug_log!();
            return Err((
                ERR_MALFORMED_INPUT.to_le_bytes().into(),
                ExitCode::PrecompileError,
            ));
        }
    };
    debug_log!();
    match result {
        ResultOrInt::Result(r) => match r {
            Ok(v) => {
                // sdk.write(&ExitCode::Ok.into_i32().to_le_bytes());
                debug_log!("v: {:x?}", &v);
                // sdk.write(&v);
                return Ok(v);
            }
            Err(_) => {
                debug_log!("failed to exec: unknown error");
                panic!("failed to exec: unknown error")
            }
        },
        ResultOrInt::Interruption() => {
            debug_log!("interruption");
            print_stats();
            if try_process_read_query_batch::<true, false>(sdk) {
                debug_log!();
                return Err((Bytes::new(), ExitCode::InterruptionCalled));
            };
            debug_log!();
        }
    }
    Ok(Bytes::new())
}

system_entrypoint2!(main_entry, deploy_entry);
