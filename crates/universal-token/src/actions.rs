use crate::consts::{
    ERR_ALREADY_PAUSED, ERR_ALREADY_UNPAUSED, ERR_INSUFFICIENT_ALLOWANCE, ERR_INVALID_MINTER,
    ERR_INVALID_PAUSER, ERR_INVALID_RECIPIENT, ERR_MINTABLE_PLUGIN_NOT_ACTIVE, ERR_OVERFLOW,
    ERR_PAUSABLE_PLUGIN_NOT_ACTIVE,
};
use crate::events::{
    emit_approval_event, emit_pause_event, emit_transfer_event, emit_unpause_event,
};
use crate::storage::{Allowance, Balance, Config, Settings};
use crate::{evm_exit, return_error_if_false};
use alloc::vec::Vec;
use fluentbase_sdk::{Address, SharedAPI, U256};
use solana_pubkey::Pubkey;

pub fn get_symbol<SDK: SharedAPI>(sdk: &SDK) -> Vec<u8> {
    Settings::symbol(sdk)
}
pub fn get_name<SDK: SharedAPI>(sdk: &SDK) -> Vec<u8> {
    Settings::name(sdk)
}
pub fn get_decimals<SDK: SharedAPI>(sdk: &SDK) -> U256 {
    Settings::decimals_get(sdk)
}

pub fn do_transfer<SDK: SharedAPI>(
    sdk: &mut SDK,
    from: Address,
    to: Address,
    amount: U256,
) -> Result<(), u32> {
    Balance::send(sdk, from, to, amount)?;
    emit_transfer_event(sdk, &from, &to, &amount);
    Ok(())
}

pub fn do_transfer_from<SDK: SharedAPI>(
    sdk: &mut SDK,
    spender: Address,
    from: Address,
    to: Address,
    amount: U256,
) -> Result<(), u32> {
    return_error_if_false!(
        Allowance::subtract(sdk, from, spender, amount),
        ERR_INSUFFICIENT_ALLOWANCE
    );
    Balance::send(sdk, from, to, amount)?;
    emit_transfer_event(sdk, &from, &to, &amount);
    Ok(())
}

pub fn do_approve<SDK: SharedAPI>(
    sdk: &mut SDK,
    owner: &Address,
    spender: &Address,
    amount: &U256,
) {
    Allowance::update(sdk, *owner, *spender, *amount);
    emit_approval_event(sdk, &owner, &spender, &amount);
}

pub fn get_allowance<SDK: SharedAPI>(sdk: &mut SDK, owner: &Address, spender: &Address) -> U256 {
    Allowance::get_current(sdk, *owner, *spender)
}

pub fn get_total_supply<SDK: SharedAPI>(sdk: &SDK) -> U256 {
    Settings::total_supply_get(sdk)
}

pub fn get_balance_of<SDK: SharedAPI>(sdk: &SDK, address: &Address) -> U256 {
    Balance::get_for(sdk, *address)
}

pub fn do_mint<SDK: SharedAPI>(
    sdk: &mut SDK,
    config: &mut Config,
    minter: &Address,
    to: &Address,
    amount: &U256,
) -> Result<(), u32> {
    if !config.mintable_plugin_enabled(sdk) {
        return Err(ERR_MINTABLE_PLUGIN_NOT_ACTIVE);
    }
    if minter != &Settings::minter_get(sdk) {
        return Err(ERR_INVALID_MINTER);
    }
    if config.pausable_plugin_enabled(sdk) && config.paused(sdk) {
        return Err(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    let zero_address = Address::ZERO;
    if to == &zero_address {
        return Err(ERR_INVALID_RECIPIENT);
    }
    let total_supply = Settings::total_supply_get(sdk);
    let (total_supply, overflow) = total_supply.overflowing_add(*amount);
    if overflow {
        return Err(ERR_OVERFLOW);
    }
    Settings::total_supply_set(sdk, total_supply);
    evm_exit!(sdk, Balance::add(sdk, *to, *amount));
    emit_transfer_event(sdk, &zero_address, &to, &amount);
    Ok(())
}

pub fn do_pause<SDK: SharedAPI>(
    sdk: &mut SDK,
    config: &mut Config,
    pauser: &Address,
) -> Result<(), u32> {
    if !config.pausable_plugin_enabled(sdk) {
        return Err(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    if pauser != &Settings::pauser_get(sdk) {
        return Err(ERR_INVALID_PAUSER);
    }
    if config.paused(sdk) {
        return Err(ERR_ALREADY_PAUSED);
    }
    config.pause(sdk);
    config.save_flags(sdk);
    emit_pause_event(sdk, &pauser);
    Ok(())
}

pub fn do_unpause<SDK: SharedAPI>(
    sdk: &mut SDK,
    config: &mut Config,
    pauser: &Address,
) -> Result<(), u32> {
    if !config.pausable_plugin_enabled(sdk) {
        return Err(ERR_PAUSABLE_PLUGIN_NOT_ACTIVE);
    }
    if pauser != &Settings::pauser_get(sdk) {
        return Err(ERR_INVALID_PAUSER);
    }
    if !config.paused(sdk) {
        return Err(ERR_ALREADY_UNPAUSED);
    }
    config.unpause(sdk);
    config.save_flags(sdk);
    emit_unpause_event(sdk, &pauser);
    Ok(())
}
