#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
//! Universal Token: an ERC-20–style token implementation for Fluentbase.
//!
//! The contract exposes a selector-based ABI (4-byte big-endian selectors) and stores balances, allowances,
//! and optional plugin configuration in Fluentbase storage.

extern crate alloc;
extern crate core;

#[cfg(test)]
mod tests;

use fluentbase_sdk::{
    evm::write_evm_exit_message,
    storage::{StorageMap, StorageU256},
    system_entrypoint2, Address, ContextReader, EvmExitCode, ExitCode, SharedAPI, StorageUtils,
    U256,
};
use fluentbase_universal_token::{
    command::{
        AllowanceCommand, ApproveCommand, BalanceOfCommand, MintCommand, TransferCommand,
        TransferFromCommand, UniversalTokenCommand,
    },
    consts::*,
    events::{emit_approval_event, emit_pause_event, emit_transfer_event, emit_unpause_event},
    storage::{InitialSettings, SIG_LEN_BYTES},
};

/// Balance mapping: `owner -> balance`.
type BalanceStorageMap = StorageMap<Address, StorageU256>;
/// Allowance mapping: `owner -> (spender -> allowance)`.
type AllowanceStorageMap = StorageMap<Address, StorageMap<Address, StorageU256>>;

/// Returns the ERC-20 `symbol()` as a short string stored at `SYMBOL_STORAGE_SLOT`.
fn erc20_symbol_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let value = sdk.storage_short_string(&SYMBOL_STORAGE_SLOT)?;
    sdk.write(value.as_bytes());
    Ok(0)
}

/// Returns the ERC-20 `name()` as a short string stored at `NAME_STORAGE_SLOT`.
fn erc20_name_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let value = sdk.storage_short_string(&NAME_STORAGE_SLOT)?;
    sdk.write(value.as_bytes());
    Ok(0)
}

/// Returns the ERC-20 `decimals()` as a 32-byte big-endian U256 word.
fn erc20_decimals_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let value = sdk.storage(&DECIMALS_STORAGE_SLOT).ok()?;
    let value = value.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&value);
    Ok(0)
}

/// Implements ERC-20 `transfer(to, amount)` using the caller as the sender.
fn erc20_transfer_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_MINTING_PAUSED);
    }

    let from = sdk.context().contract_caller();
    let TransferCommand { to, amount } = TransferCommand::try_decode(input)?;

    let balance_storage_map = BalanceStorageMap::new(BALANCE_STORAGE_SLOT);

    // Read current state first so we can fail without mutating storage.
    let sender_accessor = balance_storage_map.entry(from);
    let sender_balance = sender_accessor.get_checked(sdk)?;
    let Some(new_sender_balance) = sender_balance.checked_sub(amount) else {
        return Ok(ERR_INSUFFICIENT_BALANCE);
    };

    let recipient_accessor = balance_storage_map.entry(to);
    let recipient_balance = recipient_accessor.get_checked(sdk)?;
    let Some(new_recipient_balance) = recipient_balance.checked_add(amount) else {
        return Ok(ERR_INTEGER_OVERFLOW);
    };

    // Commit state changes only after all checks pass.
    sender_accessor.set_checked(sdk, new_sender_balance)?;
    recipient_accessor.set_checked(sdk, new_recipient_balance)?;

    emit_transfer_event(sdk, &from, &to, &amount)?;

    let output = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&output);
    Ok(0)
}

/// Implements ERC-20 `transferFrom(from, to, amount)` using caller as the spender.
fn erc20_transfer_from_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_MINTING_PAUSED);
    }

    let spender = sdk.context().contract_caller();
    let TransferFromCommand { from, to, amount } = TransferFromCommand::try_decode(input)?;

    let allowance_storage_map = AllowanceStorageMap::new(ALLOWANCE_STORAGE_SLOT);
    let balance_storage_map = BalanceStorageMap::new(BALANCE_STORAGE_SLOT);

    // Read all state first; do not mutate on failure.
    let allowance_accessor = allowance_storage_map.entry(from).entry(spender);
    let allowance = allowance_accessor.get_checked(sdk)?;
    let Some(new_allowance) = allowance.checked_sub(amount) else {
        return Ok(ERR_INSUFFICIENT_ALLOWANCE);
    };

    let sender_accessor = balance_storage_map.entry(from);
    let sender_balance = sender_accessor.get_checked(sdk)?;
    let Some(new_sender_balance) = sender_balance.checked_sub(amount) else {
        return Ok(ERR_INSUFFICIENT_BALANCE);
    };

    let recipient_accessor = balance_storage_map.entry(to);
    let recipient_balance = recipient_accessor.get_checked(sdk)?;
    let Some(new_recipient_balance) = recipient_balance.checked_add(amount) else {
        return Ok(ERR_INTEGER_OVERFLOW);
    };

    // Commit state only after all checks pass.
    allowance_accessor.set_checked(sdk, new_allowance)?;
    sender_accessor.set_checked(sdk, new_sender_balance)?;
    recipient_accessor.set_checked(sdk, new_recipient_balance)?;

    emit_transfer_event(sdk, &from, &to, &amount)?;

    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&result);
    Ok(0)
}

/// Implements ERC-20 `approve(spender, amount)` / allowance update (see note: adds to existing allowance).
fn erc20_approve_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let contract_caller = sdk.context().contract_caller();
    let ApproveCommand { spender, amount } = ApproveCommand::try_decode(input)?;

    let allowance_accessor = AllowanceStorageMap::new(ALLOWANCE_STORAGE_SLOT)
        .entry(contract_caller)
        .entry(spender);
    allowance_accessor.set_checked(sdk, amount)?;

    emit_approval_event(sdk, &contract_caller, &spender, &amount)?;

    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&result);
    Ok(0)
}

/// Returns ERC-20 `allowance(owner, spender)` as a 32-byte big-endian U256 word.
fn erc20_allowance_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let AllowanceCommand { owner, spender } = AllowanceCommand::try_decode(input)?;
    let result = AllowanceStorageMap::new(ALLOWANCE_STORAGE_SLOT)
        .entry(owner)
        .entry(spender)
        .get_checked(sdk)?
        .to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&result);
    Ok(0)
}

/// Returns ERC-20 `totalSupply()` as a 32-byte big-endian U256 word.
fn erc20_total_supply_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let value = sdk.storage(&TOTAL_SUPPLY_STORAGE_SLOT).ok()?;
    let value = value.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&value);
    Ok(0)
}

/// Returns the caller's balance (convenience method) as a 32-byte big-endian U256 word.
fn erc20_balance_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let caller = sdk.context().contract_caller();
    let balance = BalanceStorageMap::new(BALANCE_STORAGE_SLOT)
        .entry(caller)
        .get_checked(sdk)?
        .to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&balance);
    Ok(0)
}

/// Returns `balanceOf(owner)` as a 32-byte big-endian U256 word.
fn erc20_balance_of_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let BalanceOfCommand { owner } = BalanceOfCommand::try_decode(input)?;
    let balance = BalanceStorageMap::new(BALANCE_STORAGE_SLOT)
        .entry(owner)
        .get_checked(sdk)?
        .to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&balance);
    Ok(0)
}

/// Mints tokens when the mintable plugin is enabled and the caller is the configured minter.
fn erc20_mint_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let contract_minter = sdk.storage_address(&MINTER_STORAGE_SLOT)?;
    if contract_minter == Address::ZERO {
        return Ok(ERR_CONTRACT_NOT_MINTABLE);
    }
    let caller = sdk.context().contract_caller();
    if caller != contract_minter {
        return Ok(ERR_INVALID_MINTER);
    }

    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_MINTING_PAUSED);
    }

    let MintCommand { to, amount } = MintCommand::try_decode(input)?;
    if to == Address::ZERO {
        return Ok(ERR_INVALID_RECIPIENT);
    }

    // Read current state first so we can fail without partial writes.
    let total_supply = sdk.storage(&TOTAL_SUPPLY_STORAGE_SLOT).ok()?;
    let Some(new_total_supply) = total_supply.checked_add(amount) else {
        return Ok(ERR_INTEGER_OVERFLOW);
    };

    let recipient_accessor = BalanceStorageMap::new(BALANCE_STORAGE_SLOT).entry(to);
    let recipient_balance = recipient_accessor.get_checked(sdk)?;
    let Some(new_recipient_balance) = recipient_balance.checked_add(amount) else {
        return Ok(ERR_INTEGER_OVERFLOW);
    };

    // Commit state.
    sdk.write_storage(TOTAL_SUPPLY_STORAGE_SLOT, new_total_supply)
        .ok()?;
    recipient_accessor.set_checked(sdk, new_recipient_balance)?;

    emit_transfer_event(sdk, &Address::ZERO, &to, &amount)?;

    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&result);
    Ok(0)
}

/// Pauses transfers/minting when the pausable plugin is enabled and the caller is the configured pauser.
fn erc20_pause_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    // Make sure contract is pausable (pauser is provided)
    let contract_pauser = sdk.storage_address(&PAUSER_STORAGE_SLOT)?;
    if contract_pauser.is_zero() {
        return Ok(ERR_CONTRACT_NOT_PAUSABLE);
    }
    // Make sure contract is unpaused
    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_ALREADY_PAUSED);
    }
    // Check is caller (sender) is pauser, because only pauser can pause/unpause the contract
    let contract_caller = sdk.context().contract_caller();
    if contract_caller != contract_pauser {
        return Ok(ERR_PAUSER_MISMATCH);
    }
    // Write paused flag
    sdk.write_storage(CONTRACT_FROZEN_STORAGE_SLOT, U256::ONE)
        .ok()?;
    // Emit an event, indicating that contract is paused
    emit_pause_event(sdk, &contract_caller)?;
    // Write output (1)
    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&result);
    Ok(0)
}

/// Unpauses the contract when the pausable plugin is enabled and the caller is the configured pauser.
fn erc20_unpause_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    // Make sure contract is pausable (pauser is provided)
    let contract_pauser = sdk.storage_address(&PAUSER_STORAGE_SLOT)?;
    if contract_pauser.is_zero() {
        return Ok(ERR_CONTRACT_NOT_PAUSABLE);
    }
    // Make sure contract is paused
    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if is_contract_frozen.is_zero() {
        return Ok(ERR_ALREADY_UNPAUSED);
    }
    // Check is caller (sender) is pauser, because only pauser can pause/unpause the contract
    let contract_caller = sdk.context().contract_caller();
    if contract_caller != contract_pauser {
        return Ok(ERR_PAUSER_MISMATCH);
    }
    // Write paused flag
    sdk.write_storage(CONTRACT_FROZEN_STORAGE_SLOT, U256::ZERO)
        .ok()?;
    // Emit an event indicating contract is now unpaused
    emit_unpause_event(sdk, &contract_caller)?;
    // Write success (1)
    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(&result);
    Ok(0)
}

/// Fallback for unknown selectors: returns `ERR_UNKNOWN_METHOD`.
fn erc20_unknown_method<SDK: SharedAPI>(
    _sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    Ok(ERR_UNKNOWN_METHOD)
}

/// Constructor entrypoint: decodes `InitialSettings` and initializes storage (metadata, supply, optional minter/pauser).
fn erc20_constructor_handler<SDK: SharedAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    // Decode initial settings parameters (SolidityABI)
    let InitialSettings {
        token_name,
        token_symbol,
        decimals,
        initial_supply,
        minter,
        pauser,
    } = InitialSettings::decode_with_prefix(input).ok_or(ExitCode::MalformedBuiltinParams)?;
    // Write token name and token decimals (make sure both are properly UTF-8 encoded)
    sdk.write_storage_short_string(
        NAME_STORAGE_SLOT,
        token_name
            .as_str()
            .ok_or(ExitCode::MalformedBuiltinParams)?,
    )?;
    sdk.write_storage_short_string(
        SYMBOL_STORAGE_SLOT,
        token_symbol
            .as_str()
            .ok_or(ExitCode::MalformedBuiltinParams)?,
    )?;
    // We should store decimals in the storage
    sdk.write_storage(DECIMALS_STORAGE_SLOT, U256::from(decimals))
        .ok()?;
    // Mint required tokens to sender based on the initial supply
    if initial_supply > 0 {
        let caller = sdk.context().contract_caller();
        // Assign caller balance
        BalanceStorageMap::new(BALANCE_STORAGE_SLOT)
            .entry(caller)
            .set_checked(sdk, initial_supply)?;
        // Increase token supply
        sdk.write_storage(TOTAL_SUPPLY_STORAGE_SLOT, initial_supply)
            .ok()?;
    }
    // If token is mintable then minter is provided
    if let Some(minter) = minter {
        sdk.write_storage_address(MINTER_STORAGE_SLOT, minter)?;
    }
    // If token is pausable then pauser is provided
    if let Some(pauser) = pauser {
        sdk.write_storage_address(PAUSER_STORAGE_SLOT, pauser)?;
    }
    Ok(0)
}

#[inline(never)]
pub fn deploy_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    let input = sdk.input();
    let evm_exit_code = erc20_constructor_handler(sdk, input)?;
    if evm_exit_code != 0 {
        write_evm_exit_message(evm_exit_code, |slice| sdk.write(slice));
        return Err(ExitCode::Panic);
    }
    Ok(())
}

pub fn main_entry<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    let (sig, input) = sdk.input().split_at(SIG_LEN_BYTES);
    let sig = u32::from_be_bytes(sig.try_into().unwrap());
    let evm_exit_code = match sig {
        SIG_SYMBOL => erc20_symbol_handler(sdk, input),
        SIG_NAME => erc20_name_handler(sdk, input),
        SIG_TRANSFER => erc20_transfer_handler(sdk, input),
        SIG_TRANSFER_FROM => erc20_transfer_from_handler(sdk, input),
        SIG_APPROVE => erc20_approve_handler(sdk, input),
        SIG_DECIMALS => erc20_decimals_handler(sdk, input),
        SIG_ALLOWANCE => erc20_allowance_handler(sdk, input),
        SIG_TOTAL_SUPPLY => erc20_total_supply_handler(sdk, input),
        SIG_BALANCE => erc20_balance_handler(sdk, input),
        SIG_BALANCE_OF => erc20_balance_of_handler(sdk, input),
        SIG_MINT => erc20_mint_handler(sdk, input),
        SIG_PAUSE => erc20_pause_handler(sdk, input),
        SIG_UNPAUSE => erc20_unpause_handler(sdk, input),
        _ => erc20_unknown_method(sdk, input),
    }?;
    if evm_exit_code != 0 {
        write_evm_exit_message(evm_exit_code, |slice| sdk.write(slice));
        return Err(ExitCode::Panic);
    }
    Ok(())
}

system_entrypoint2!(main_entry, deploy_entry);
