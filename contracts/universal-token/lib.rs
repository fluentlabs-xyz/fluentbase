#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
extern crate alloc;
extern crate core;

use fluentbase_sdk::bincode::Encode;
use fluentbase_sdk::{
    system_entrypoint2, Address, Bytes, ContextReader, ExitCode,
    RuntimeUniversalTokenNewFrameInputV1, RuntimeUniversalTokenOutputV1, SharedAPI, U256,
};
use fluentbase_universal_token::consts::{ERR_INVALID_INPUT, ERR_MINTING_PAUSED};
use fluentbase_universal_token::events::{
    emit_approval_event, emit_pause_event, emit_transfer_event, emit_unpause_event,
};
use fluentbase_universal_token::helpers::bincode::{decode, encode};
use fluentbase_universal_token::services::global_service::global_service;
use fluentbase_universal_token::storage::{allowance_service, balance_service, settings_service};
use fluentbase_universal_token::types::input_commands::{
    AllowanceCommand, ApproveCommand, BalanceOfCommand, Encodable, MintCommand, TransferCommand,
    TransferFromCommand,
};
use fluentbase_universal_token::{
    common::{fixed_bytes_from_u256, sig_from_slice, u256_from_fixed_bytes},
    consts::{
        ERR_ALREADY_PAUSED, ERR_ALREADY_UNPAUSED, ERR_DECODE, ERR_INVALID_META_NAME,
        ERR_INVALID_META_SYMBOL, ERR_INVALID_MINTER, ERR_MALFORMED_INPUT,
        ERR_MINTABLE_PLUGIN_NOT_ACTIVE, ERR_OVERFLOW, ERR_PAUSABLE_PLUGIN_NOT_ACTIVE,
        ERR_PAUSER_MISMATCH, ERR_VALIDATION, SIG_ALLOWANCE, SIG_APPROVE, SIG_BALANCE,
        SIG_BALANCE_OF, SIG_DECIMALS, SIG_MINT, SIG_NAME, SIG_PAUSE, SIG_SYMBOL, SIG_TOTAL_SUPPLY,
        SIG_TRANSFER, SIG_TRANSFER_FROM, SIG_UNPAUSE,
    },
    storage::{Config, Feature, InitialSettings, SIG_LEN_BYTES},
};

macro_rules! custom_err_tuple {
    ($e:ident) => {
        ($e.to_le_bytes().into(), ExitCode::Panic)
    };
}

macro_rules! return_custom_err {
    ($e:ident) => {
        return Err(custom_err_tuple!($e));
    };
}

fn symbol(_input: &[u8]) -> Result<Bytes, u32> {
    Ok(settings_service()
        .symbol()
        .expect("symbol value exists")
        .into())
}
fn name(_input: &[u8]) -> Result<Bytes, u32> {
    Ok(settings_service().name().expect("name value exists").into())
}
fn decimals(_input: &[u8]) -> Result<Bytes, u32> {
    Ok(fixed_bytes_from_u256(
        &settings_service()
            .decimals_get()
            .expect("decimals value exists"),
    )
    .into())
}

fn transfer(sdk: &mut impl SharedAPI, input: &[u8]) -> Result<Bytes, u32> {
    let from = sdk.context().contract_caller();
    let c = TransferCommand::try_decode(input)?;
    balance_service().send(&from, &c.to, &c.amount)?;
    emit_transfer_event(&from, &c.to, &c.amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    Ok(result)
}

fn transfer_from(input: &[u8]) -> Result<Bytes, u32> {
    let c = TransferFromCommand::try_decode(input)?;
    allowance_service().subtract(&c.from, &c.to, &c.amount)?;
    balance_service().send(&c.from, &c.to, &c.amount)?;
    emit_transfer_event(&c.from, &c.to, &c.amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    Ok(result)
}

fn approve(input: &[u8]) -> Result<Bytes, u32> {
    let c = ApproveCommand::try_decode(input)?;
    allowance_service().update(&c.owner, &c.spender, &c.amount);
    emit_approval_event(&c.owner, &c.spender, &c.amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    Ok(result)
}

fn allowance(input: &[u8]) -> Result<Bytes, u32> {
    let c = AllowanceCommand::try_decode(input)?;
    let amount = allowance_service().get(&c.owner, &c.spender);
    let bytes: Bytes = fixed_bytes_from_u256(&amount).into();
    Ok(bytes)
}

fn total_supply(_input: &[u8]) -> Result<Bytes, u32> {
    let result = settings_service()
        .total_supply_get()
        .expect("total_supply value exists");
    let result: Bytes = fixed_bytes_from_u256(&result).into();
    Ok(result)
}

fn balance(sdk: &mut impl SharedAPI, _input: &[u8]) -> Result<Bytes, u32> {
    let result = balance_service().get(&sdk.context().contract_caller());
    let result: Bytes = fixed_bytes_from_u256(&result).into();
    Ok(result)
}

fn balance_of(input: &[u8]) -> Result<Bytes, u32> {
    let c = BalanceOfCommand::try_decode(input)?;
    let result = balance_service().get(&c.owner);
    let result: Bytes = fixed_bytes_from_u256(&result).into();
    Ok(result)
}

fn mint(sdk: &mut impl SharedAPI, input: &[u8]) -> Result<Bytes, u32> {
    let mut config = Config::new();
    if !config.mintable_plugin_enabled()? {
        return Err(ERR_MINTABLE_PLUGIN_NOT_ACTIVE);
    }
    let minter = sdk.context().contract_caller();
    if minter != settings_service().minter_get().expect("minter exists") {
        return Err(ERR_INVALID_MINTER);
    }
    if config.pausable_plugin_enabled()? && config.paused()? {
        return Err(ERR_MINTING_PAUSED);
    }
    let c = MintCommand::try_decode(input)?;
    let total_supply = settings_service()
        .total_supply_get()
        .expect("total supply exists");
    let (total_supply, overflow) = total_supply.overflowing_add(c.amount);
    if overflow {
        return Err(ERR_OVERFLOW);
    }
    settings_service().total_supply_set(&total_supply);
    balance_service().add(&c.to, &c.amount)?;
    emit_transfer_event(&Address::ZERO, &c.to, &c.amount);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    Ok(result)
}

fn pause(sdk: &mut impl SharedAPI, _input: &[u8]) -> Result<Bytes, u32> {
    let mut config = Config::new();
    if !config.pausable_plugin_enabled()? {
        return Err(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let pauser = sdk.context().contract_caller();
    let current_pauser: Address = settings_service().pauser_get().expect("pauser exists");
    if pauser != current_pauser {
        return Err(ERR_PAUSER_MISMATCH);
    }
    if config.paused()? {
        return Err(ERR_ALREADY_PAUSED);
    }
    config.pause();
    config.save_flags();
    emit_pause_event(&pauser);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    Ok(result)
}

fn unpause(sdk: &mut impl SharedAPI, _input: &[u8]) -> Result<Bytes, u32> {
    let mut config = Config::new();
    if !config.pausable_plugin_enabled()? {
        return Err(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let pauser = sdk.context().contract_caller();
    let current_pauser: Address = settings_service().pauser_get().expect("pauser exists");
    if pauser != current_pauser {
        return Err(ERR_PAUSER_MISMATCH);
    }
    if !config.paused()? {
        return Err(ERR_ALREADY_UNPAUSED);
    }
    config.unpause();
    config.save_flags();
    emit_unpause_event(&pauser);
    let result: Bytes = fixed_bytes_from_u256(&U256::from(1)).into();
    Ok(result)
}

#[inline(never)]
pub fn deploy_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<Bytes, (Bytes, ExitCode)> {
    let input = sdk.bytes_input();
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        return_custom_err!(ERR_MALFORMED_INPUT)
    }

    let (new_frame_input, _) = decode::<RuntimeUniversalTokenNewFrameInputV1>(&input).unwrap();
    new_frame_input.storage.iter().for_each(|(k, v)| {
        global_service().set_value(
            &U256::from_le_slice(k.as_slice()),
            &U256::from_le_slice(v.as_slice()),
        );
    });

    let (_sig, input) = new_frame_input.input.split_at(SIG_LEN_BYTES);
    let (initial_settings, _) = InitialSettings::try_decode_from_slice(&input)
        .map_err(|_| custom_err_tuple!(ERR_DECODE))?;

    if !initial_settings.is_valid() {
        return_custom_err!(ERR_VALIDATION);
    }
    let mut config = Config::new();
    for feature in initial_settings.features() {
        let result: Result<(), u32> = match feature {
            Feature::Meta { name, symbol } => {
                if !settings_service().name_set(name) {
                    return_custom_err!(ERR_INVALID_META_NAME);
                }
                if !settings_service().symbol_set(symbol) {
                    return_custom_err!(ERR_INVALID_META_SYMBOL);
                }
                Ok(())
            }
            Feature::InitialSupply {
                amount,
                owner,
                decimals,
            } => {
                let amount = u256_from_fixed_bytes(amount);
                let owner = owner.into();
                if !settings_service().decimals_set(*decimals) {
                    return_custom_err!(ERR_INVALID_INPUT);
                };
                settings_service().total_supply_set(&amount);
                balance_service().add(&owner, &amount)
            }
            Feature::Mintable { minter } => {
                config.enable_mintable_plugin();
                settings_service().minter_set(&Address::from(minter));
                Ok(())
            }
            Feature::Pausable { pauser } => {
                config.enable_pausable_plugin();
                settings_service().pauser_set(&Address::from(pauser));
                Ok(())
            }
        };
        match result {
            Ok(_) => {}
            Err(e) => {
                return_custom_err!(e);
            }
        }
    }
    config.save_flags();
    sdk.write(&ExitCode::Ok.into_i32().to_le_bytes());
    let output = encode(&RuntimeUniversalTokenOutputV1 {
        storage: global_service()
            .new_values()
            .iter()
            .map(|(k, v)| (k.to_le_bytes(), v.to_le_bytes()))
            .collect(),
        ..Default::default()
    })
    .unwrap();

    sdk.write(&output);
    global_service().clear();
    Ok(Bytes::new())
}

#[inline(never)]
pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<Bytes, (Bytes, ExitCode)> {
    let input = sdk.input();
    let (new_frame_input, _) = decode::<RuntimeUniversalTokenNewFrameInputV1>(input).unwrap();

    new_frame_input.storage.iter().for_each(|(k, v)| {
        global_service().set_value(
            &U256::from_le_slice(k.as_slice()),
            &U256::from_le_slice(v.as_slice()),
        );
    });
    let input_size = new_frame_input.input.len() as u32;
    if input_size < SIG_LEN_BYTES as u32 {
        return_custom_err!(ERR_MALFORMED_INPUT);
    }
    let (sig_bytes, input) = new_frame_input.input.split_at(SIG_LEN_BYTES);
    let signature = sig_from_slice(sig_bytes).unwrap();
    let result: Result<Bytes, u32> = match signature {
        SIG_SYMBOL => symbol(input),
        SIG_NAME => name(input),
        SIG_TRANSFER => transfer(sdk, input),
        SIG_TRANSFER_FROM => transfer_from(input),
        SIG_APPROVE => approve(input),
        SIG_DECIMALS => decimals(input),
        SIG_ALLOWANCE => allowance(input),
        SIG_TOTAL_SUPPLY => total_supply(input),
        SIG_BALANCE => balance(sdk, input),
        SIG_BALANCE_OF => balance_of(input),
        SIG_MINT => mint(sdk, input),
        SIG_PAUSE => pause(sdk, input),
        SIG_UNPAUSE => unpause(sdk, input),
        _ => {
            return_custom_err!(ERR_MALFORMED_INPUT)
        }
    };
    match result {
        Ok(v) => {
            let output = encode(&RuntimeUniversalTokenOutputV1 {
                output: v.into(),
                storage: {
                    let result = global_service()
                        .new_values()
                        .iter()
                        .map(|(k, v)| (k.to_le_bytes(), v.to_le_bytes()))
                        .collect();
                    global_service().clear();
                    result
                },
                events: global_service().take_events(),
            })
            .unwrap();
            Ok(output.into())
        }
        Err(e) => {
            global_service().clear();
            return_custom_err!(e)
        }
    }
}

system_entrypoint2!(main_entry, deploy_entry);
