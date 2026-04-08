#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
//! Universal Token: an ERC-20–style token implementation for Fluentbase.
//!
//! The contract exposes a selector-based ABI (4-byte big-endian selectors) and stores balances, allowances,
//! and optional plugin configuration in Fluentbase storage.

extern crate alloc;
extern crate core;

#[cfg(test)]
mod tests;

use alloc::vec::Vec;
use fluentbase_sdk::{
    bytes::BytesMut,
    codec::SolidityABI,
    crypto::crypto_keccak256,
    derive::Event,
    evm::write_evm_exit_message,
    storage::{StorageMap, StorageU256},
    system_entrypoint,
    universal_token::*,
    Address, B256, B512, ContextReader, EvmExitCode, ExitCode, StorageUtils, SystemAPI, U256,
};
use revm_precompile::secp256k1::ecrecover;

mod events {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Event)]
    pub struct Transfer {
        #[indexed]
        pub from: Address,
        #[indexed]
        pub to: Address,
        pub amount: U256,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Event)]
    pub struct Approval {
        #[indexed]
        pub owner: Address,
        #[indexed]
        pub spender: Address,
        pub amount: U256,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Event)]
    pub struct Paused {
        pub pauser: Address,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Event)]
    pub struct Unpaused {
        pub pauser: Address,
    }
}

/// Balance mapping: `owner -> balance`.
type BalanceStorageMap = StorageMap<Address, StorageU256>;
/// Allowance mapping: `owner -> (spender -> allowance)`.
type AllowanceStorageMap = StorageMap<Address, StorageMap<Address, StorageU256>>;
/// Permit nonce mapping: `owner -> nonce`.
type NonceStorageMap = StorageMap<Address, StorageU256>;

#[inline(always)]
fn nonce_get<SDK: SystemAPI>(sdk: &mut SDK, owner: Address) -> Result<U256, ExitCode> {
    Ok(NonceStorageMap::new(NONCES_STORAGE_SLOT).entry(owner).get(sdk))
}

#[inline(always)]
fn nonce_set<SDK: SystemAPI>(sdk: &mut SDK, owner: Address, nonce: U256) -> Result<(), ExitCode> {
    NonceStorageMap::new(NONCES_STORAGE_SLOT)
        .entry(owner)
        .set_checked(sdk, nonce)
}

/// Returns the ERC-20 `symbol()` as a short string stored at `SYMBOL_STORAGE_SLOT`.
fn erc20_symbol_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let value = sdk.storage_short_string(&SYMBOL_STORAGE_SLOT)?;
    let mut bytes = BytesMut::new();
    SolidityABI::encode(&value, &mut bytes, 0).unwrap();
    let result = bytes.freeze();
    sdk.write(result);
    Ok(0)
}

/// Returns the ERC-20 `name()` as a short string stored at `NAME_STORAGE_SLOT`.
fn erc20_name_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let value = sdk.storage_short_string(&NAME_STORAGE_SLOT)?;
    let mut bytes = BytesMut::new();
    SolidityABI::encode(&value, &mut bytes, 0).unwrap();
    let result = bytes.freeze();
    sdk.write(result);
    Ok(0)
}

/// Returns the ERC-20 `decimals()` as a 32-byte big-endian U256 word.
fn erc20_decimals_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let value = sdk.storage(&DECIMALS_STORAGE_SLOT).ok()?;
    let value = value.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(value);
    Ok(0)
}

/// Implements ERC-20 `transfer(to, amount)` using the caller as the sender.
fn erc20_transfer_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_PAUSABLE_ENFORCED_PAUSE);
    }

    let from = sdk.context().contract_caller();
    if from.is_zero() {
        return Ok(ERR_ERC20_INVALID_SENDER);
    }
    let TransferCommand { to, amount } = TransferCommand::try_decode(input)?;
    if to.is_zero() {
        return Ok(ERR_ERC20_INVALID_RECEIVER);
    }

    let balance_storage_map = BalanceStorageMap::new(BALANCE_STORAGE_SLOT);

    // Read current state first so we can fail without mutating storage.
    let sender_accessor = balance_storage_map.entry(from);
    let sender_balance = sender_accessor.get_checked(sdk)?;
    let Some(new_sender_balance) = sender_balance.checked_sub(amount) else {
        return Ok(ERR_ERC20_INSUFFICIENT_BALANCE);
    };
    sender_accessor.set_checked(sdk, new_sender_balance)?;

    let recipient_accessor = balance_storage_map.entry(to);
    let recipient_balance = recipient_accessor.get_checked(sdk)?;
    let new_recipient_balance = recipient_balance
        .checked_add(amount)
        .ok_or(ExitCode::IntegerOverflow)?;
    recipient_accessor.set_checked(sdk, new_recipient_balance)?;

    events::Transfer { from, to, amount }.emit(sdk)?;

    let output = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(output);
    Ok(0)
}

/// Implements ERC-20 `transferFrom(from, to, amount)` using caller as the spender.
fn erc20_transfer_from_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_PAUSABLE_ENFORCED_PAUSE);
    }

    let spender = sdk.context().contract_caller();
    let TransferFromCommand { from, to, amount } = TransferFromCommand::try_decode(input)?;
    if to.is_zero() {
        return Ok(ERR_ERC20_INVALID_RECEIVER);
    }

    let allowance_storage_map = AllowanceStorageMap::new(ALLOWANCE_STORAGE_SLOT);
    let balance_storage_map = BalanceStorageMap::new(BALANCE_STORAGE_SLOT);

    // Read all state first; do not mutate on failure.
    let allowance_accessor = allowance_storage_map.entry(from).entry(spender);
    let allowance = allowance_accessor.get_checked(sdk)?;
    if allowance < U256::MAX {
        let Some(new_allowance) = allowance.checked_sub(amount) else {
            return Ok(ERR_ERC20_INSUFFICIENT_ALLOWANCE);
        };
        allowance_accessor.set_checked(sdk, new_allowance)?;
    }

    let sender_accessor = balance_storage_map.entry(from);
    let sender_balance = sender_accessor.get_checked(sdk)?;
    let Some(new_sender_balance) = sender_balance.checked_sub(amount) else {
        return Ok(ERR_ERC20_INSUFFICIENT_BALANCE);
    };
    sender_accessor.set_checked(sdk, new_sender_balance)?;

    let recipient_accessor = balance_storage_map.entry(to);
    let recipient_balance = recipient_accessor.get_checked(sdk)?;
    let new_recipient_balance = recipient_balance
        .checked_add(amount)
        .ok_or(ExitCode::IntegerOverflow)?;
    recipient_accessor.set_checked(sdk, new_recipient_balance)?;

    events::Transfer { from, to, amount }.emit(sdk)?;

    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(result);
    Ok(0)
}

/// Implements ERC-20 `approve(spender, amount)` / allowance update (see note: adds to existing allowance).
fn erc20_approve_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    let contract_caller = sdk.context().contract_caller();
    let ApproveCommand { spender, amount } = ApproveCommand::try_decode(input)?;

    let allowance_accessor = AllowanceStorageMap::new(ALLOWANCE_STORAGE_SLOT)
        .entry(contract_caller)
        .entry(spender);
    allowance_accessor.set_checked(sdk, amount)?;

    events::Approval {
        owner: contract_caller,
        spender,
        amount,
    }
    .emit(sdk)?;

    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(result);
    Ok(0)
}

/// Returns ERC-20 `allowance(owner, spender)` as a 32-byte big-endian U256 word.
fn erc20_allowance_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let AllowanceCommand { owner, spender } = AllowanceCommand::try_decode(input)?;
    let result = AllowanceStorageMap::new(ALLOWANCE_STORAGE_SLOT)
        .entry(owner)
        .entry(spender)
        .get_checked(sdk)?
        .to_be_bytes::<{ U256::BYTES }>();
    sdk.write(result);
    Ok(0)
}

fn abi_word_addr(a: Address) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a.as_ref());
    w
}

fn abi_word_u256(x: U256) -> [u8; 32] {
    x.to_be_bytes::<{ U256::BYTES }>()
}

fn erc20_domain_separator_value<SDK: SystemAPI>(sdk: &mut SDK) -> Result<B256, ExitCode> {
    let token_name = sdk.storage_short_string(&NAME_STORAGE_SLOT)?;
    let name_hash = crypto_keccak256(token_name.as_bytes());

    let mut encoded = Vec::with_capacity(32 * 5);
    encoded.extend_from_slice(&EIP712_DOMAIN_TYPEHASH);
    encoded.extend_from_slice(name_hash.as_slice());
    encoded.extend_from_slice(&EIP2612_VERSION_HASH);
    encoded.extend_from_slice(&abi_word_u256(U256::from(sdk.context().block_chain_id())));
    encoded.extend_from_slice(&abi_word_addr(sdk.context().contract_address()));

    Ok(crypto_keccak256(&encoded))
}

fn ecrecover_address(digest: B256, v: u8, r: U256, s: U256) -> Option<Address> {
    let rec_id = match v {
        27 | 28 => v - 27,
        0 | 1 => v,
        _ => return None,
    };

    let mut sig_bytes = [0u8; 64];
    sig_bytes[0..32].copy_from_slice(&abi_word_u256(r));
    sig_bytes[32..64].copy_from_slice(&abi_word_u256(s));
    let sig = <&B512>::try_from(&sig_bytes[..]).ok()?;

    let recovered = ecrecover(sig, rec_id, &digest).ok()?;
    let mut recovered_addr = [0u8; 20];
    recovered_addr.copy_from_slice(&recovered[12..32]);
    let recovered_addr = Address::from_slice(&recovered_addr);

    if recovered_addr == Address::ZERO {
        return None;
    }

    Some(recovered_addr)
}

fn erc20_domain_separator_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let domain_separator = erc20_domain_separator_value(sdk)?;
    sdk.write(*domain_separator);
    Ok(0)
}

fn erc20_nonces_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let NoncesCommand { owner } = NoncesCommand::try_decode(input)?;
    let nonce = nonce_get(sdk, owner)?;
    sdk.write(abi_word_u256(nonce));
    Ok(0)
}

fn erc20_permit_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }

    let PermitCommand {
        owner,
        spender,
        value,
        deadline,
        v,
        r,
        s,
    } = PermitCommand::try_decode(input)?;

    let now = U256::from(sdk.context().block_timestamp());
    if deadline < now {
        return Ok(ERR_UST_EXPIRED_DEADLINE);
    }

    let nonce = nonce_get(sdk, owner)?;

    let mut permit_encoded = Vec::with_capacity(32 * 6);
    permit_encoded.extend_from_slice(&EIP2612_PERMIT_TYPEHASH);
    permit_encoded.extend_from_slice(&abi_word_addr(owner));
    permit_encoded.extend_from_slice(&abi_word_addr(spender));
    permit_encoded.extend_from_slice(&abi_word_u256(value));
    permit_encoded.extend_from_slice(&abi_word_u256(nonce));
    permit_encoded.extend_from_slice(&abi_word_u256(deadline));
    let permit_hash = crypto_keccak256(&permit_encoded);

    let domain_separator = erc20_domain_separator_value(sdk)?;
    let mut digest_payload = Vec::with_capacity(66);
    digest_payload.extend_from_slice(b"\x19\x01");
    digest_payload.extend_from_slice(domain_separator.as_slice());
    digest_payload.extend_from_slice(permit_hash.as_slice());
    let digest = crypto_keccak256(&digest_payload);
    let Some(recovered) = ecrecover_address(digest, v, r, s) else {
        return Ok(ERR_UST_INVALID_SIGNATURE);
    };

    if recovered != owner {
        return Ok(ERR_UST_INVALID_SIGNATURE);
    }

    AllowanceStorageMap::new(ALLOWANCE_STORAGE_SLOT)
        .entry(owner)
        .entry(spender)
        .set_checked(sdk, value)?;

    let next_nonce = nonce.checked_add(U256::ONE).ok_or(ExitCode::IntegerOverflow)?;
    nonce_set(sdk, owner, next_nonce)?;

    events::Approval {
        owner,
        spender,
        amount: value,
    }
    .emit(sdk)?;

    Ok(0)
}

/// Returns ERC-20 `totalSupply()` as a 32-byte big-endian U256 word.
fn erc20_total_supply_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let value = sdk.storage(&TOTAL_SUPPLY_STORAGE_SLOT).ok()?;
    let value = value.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(value);
    Ok(0)
}

/// Returns the caller's balance (convenience method) as a 32-byte big-endian U256 word.
fn erc20_balance_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let caller = sdk.context().contract_caller();
    let balance = BalanceStorageMap::new(BALANCE_STORAGE_SLOT)
        .entry(caller)
        .get_checked(sdk)?
        .to_be_bytes::<{ U256::BYTES }>();
    sdk.write(balance);
    Ok(0)
}

/// Returns `balanceOf(owner)` as a 32-byte big-endian U256 word.
fn erc20_balance_of_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    let BalanceOfCommand { owner } = BalanceOfCommand::try_decode(input)?;
    let balance = BalanceStorageMap::new(BALANCE_STORAGE_SLOT)
        .entry(owner)
        .get_checked(sdk)?
        .to_be_bytes::<{ U256::BYTES }>();
    sdk.write(balance);
    Ok(0)
}

/// Mints tokens when the mintable plugin is enabled and the caller is the configured minter.
fn erc20_mint_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    let contract_minter = sdk.storage_address(&MINTER_STORAGE_SLOT)?;
    if contract_minter == Address::ZERO {
        return Ok(ERR_UST_NOT_MINTABLE);
    }
    let caller = sdk.context().contract_caller();
    if caller != contract_minter {
        return Ok(ERR_UST_MINTER_MISMATCH);
    }

    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_PAUSABLE_ENFORCED_PAUSE);
    }

    let MintCommand { to, amount } = MintCommand::try_decode(input)?;
    if to == Address::ZERO {
        return Ok(ERR_ERC20_INVALID_RECEIVER);
    }

    // Read the current state first so we can fail without partial writes.
    let total_supply = sdk.storage(&TOTAL_SUPPLY_STORAGE_SLOT).ok()?;
    let new_total_supply = total_supply
        .checked_add(amount)
        .ok_or(ExitCode::IntegerOverflow)?;

    let recipient_accessor = BalanceStorageMap::new(BALANCE_STORAGE_SLOT).entry(to);
    let recipient_balance = recipient_accessor.get_checked(sdk)?;
    let new_recipient_balance = recipient_balance
        .checked_add(amount)
        .ok_or(ExitCode::IntegerOverflow)?;

    // Commit state.
    sdk.write_storage(TOTAL_SUPPLY_STORAGE_SLOT, new_total_supply)
        .ok()?;
    recipient_accessor.set_checked(sdk, new_recipient_balance)?;

    events::Transfer {
        from: Address::ZERO,
        to,
        amount,
    }
    .emit(sdk)?;

    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(result);
    Ok(0)
}

/// Burns tokens from the specified address, reducing total supply.
fn erc20_burn_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }

    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_PAUSABLE_ENFORCED_PAUSE);
    }

    // Ensure the token is configured as burnable: we currently reuse the minter role
    // as the privileged burner, mirroring how mint works for arbitrary `to`.
    let contract_minter = sdk.storage_address(&MINTER_STORAGE_SLOT)?;
    if contract_minter == Address::ZERO {
        return Ok(ERR_UST_NOT_MINTABLE);
    }

    let BurnCommand { from, amount } = BurnCommand::try_decode(input)?;
    if from.is_zero() {
        return Ok(ERR_ERC20_INVALID_SENDER);
    }

    // Only allow the configured minter (acting as burner) to burn from any `from`.
    let caller = sdk.context().contract_caller();
    if caller != contract_minter {
        return Ok(ERR_UST_MINTER_MISMATCH);
    }

    // Read current state first so we can fail without partial writes.
    let total_supply = sdk.storage(&TOTAL_SUPPLY_STORAGE_SLOT).ok()?;
    let Some(new_total_supply) = total_supply.checked_sub(amount) else {
        return Ok(ERR_ERC20_INSUFFICIENT_BALANCE);
    };

    let balance_storage_map = BalanceStorageMap::new(BALANCE_STORAGE_SLOT);
    let sender_accessor = balance_storage_map.entry(from);
    let sender_balance = sender_accessor.get_checked(sdk)?;
    let Some(new_sender_balance) = sender_balance.checked_sub(amount) else {
        return Ok(ERR_ERC20_INSUFFICIENT_BALANCE);
    };

    // Commit state.
    sdk.write_storage(TOTAL_SUPPLY_STORAGE_SLOT, new_total_supply)
        .ok()?;
    sender_accessor.set_checked(sdk, new_sender_balance)?;

    // Emit ERC20 Transfer event to zero address.
    events::Transfer {
        from,
        to: Address::ZERO,
        amount,
    }
    .emit(sdk)?;

    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(result);
    Ok(0)
}

/// Pauses transfers/minting when the pausable plugin is enabled and the caller is the configured pauser.
fn erc20_pause_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    // Make sure contract is pausable (pauser is provided)
    let contract_pauser = sdk.storage_address(&PAUSER_STORAGE_SLOT)?;
    if contract_pauser.is_zero() {
        return Ok(ERR_UST_NOT_PAUSABLE);
    }
    // Make sure the contract is unpaused
    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if !is_contract_frozen.is_zero() {
        return Ok(ERR_PAUSABLE_ENFORCED_PAUSE);
    }
    // Check is caller (sender) is pauser, because only pauser can pause/unpause the contract
    let contract_caller = sdk.context().contract_caller();
    if contract_caller != contract_pauser {
        return Ok(ERR_UST_PAUSER_MISMATCH);
    }
    // Write a paused flag
    sdk.write_storage(CONTRACT_FROZEN_STORAGE_SLOT, U256::ONE)
        .ok()?;
    // Emit an event, indicating that the contract is paused
    events::Paused {
        pauser: contract_caller,
    }
    .emit(sdk)?;
    // Write output (1)
    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(result);
    Ok(0)
}

/// Unpauses the contract when the pausable plugin is enabled and the caller is the configured pauser.
fn erc20_unpause_handler<SDK: SystemAPI>(
    sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    // Make sure contract is pausable (pauser is provided)
    let contract_pauser = sdk.storage_address(&PAUSER_STORAGE_SLOT)?;
    if contract_pauser.is_zero() {
        return Ok(ERR_UST_NOT_PAUSABLE);
    }
    // Make sure the contract is paused
    let is_contract_frozen = sdk.storage(&CONTRACT_FROZEN_STORAGE_SLOT).ok()?;
    if is_contract_frozen.is_zero() {
        return Ok(ERR_PAUSABLE_EXPECTED_PAUSE);
    }
    // Check if caller (sender) is pauser, because only pauser can pause/unpause the contract
    let contract_caller = sdk.context().contract_caller();
    if contract_caller != contract_pauser {
        return Ok(ERR_UST_PAUSER_MISMATCH);
    }
    // Write a paused flag
    sdk.write_storage(CONTRACT_FROZEN_STORAGE_SLOT, U256::ZERO)
        .ok()?;
    // Emit an event indicating a contract is now unpaused
    events::Unpaused {
        pauser: contract_caller,
    }
    .emit(sdk)?;
    // Write success (1)
    let result = U256::ONE.to_be_bytes::<{ U256::BYTES }>();
    sdk.write(result);
    Ok(0)
}

/// Fallback for unknown selectors: returns `ERR_UNKNOWN_METHOD`.
fn erc20_unknown_method<SDK: SystemAPI>(
    _sdk: &mut SDK,
    _input: &[u8],
) -> Result<EvmExitCode, ExitCode> {
    Ok(ERR_UST_UNKNOWN_METHOD)
}

/// Constructor entrypoint: decodes `InitialSettings` and initializes storage (metadata, supply, optional minter/pauser).
fn erc20_constructor_handler<SDK: SystemAPI>(
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
        // Emit transfer event
        events::Transfer {
            from: Address::ZERO,
            to: caller,
            amount: initial_supply,
        }
        .emit(sdk)?;
    }
    // If token is mintable then minter is provided
    if !minter.is_zero() {
        sdk.write_storage_address(MINTER_STORAGE_SLOT, minter)?;
    }
    // If token is pausable then pauser is provided
    if !pauser.is_zero() {
        sdk.write_storage_address(PAUSER_STORAGE_SLOT, pauser)?;
    }
    Ok(0)
}

pub fn deploy_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    let input = sdk.bytes_input();
    let evm_exit_code = erc20_constructor_handler(sdk, input.as_ref())?;
    if evm_exit_code != 0 {
        write_evm_exit_message(evm_exit_code, |slice| sdk.write(slice));
        return Err(ExitCode::Panic);
    }
    Ok(())
}

pub fn main_entry<SDK: SystemAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    let input_size = sdk.input_size();
    if input_size < SIG_LEN_BYTES as u32 {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    let input = sdk.bytes_input();
    let (sig, input) = input.split_at(SIG_LEN_BYTES);
    let sig = u32::from_be_bytes(sig.try_into().unwrap());
    let evm_exit_code = match sig {
        SIG_ERC20_SYMBOL => erc20_symbol_handler(sdk, input),
        SIG_ERC20_NAME => erc20_name_handler(sdk, input),
        SIG_ERC20_TRANSFER => erc20_transfer_handler(sdk, input),
        SIG_ERC20_TRANSFER_FROM => erc20_transfer_from_handler(sdk, input),
        SIG_ERC20_APPROVE => erc20_approve_handler(sdk, input),
        SIG_ERC20_DECIMALS => erc20_decimals_handler(sdk, input),
        SIG_ERC20_ALLOWANCE => erc20_allowance_handler(sdk, input),
        SIG_ERC20_TOTAL_SUPPLY => erc20_total_supply_handler(sdk, input),
        SIG_ERC20_BALANCE => erc20_balance_handler(sdk, input),
        SIG_ERC20_BALANCE_OF => erc20_balance_of_handler(sdk, input),
        SIG_ERC20_MINT => erc20_mint_handler(sdk, input),
        SIG_ERC20_BURN => erc20_burn_handler(sdk, input),
        SIG_ERC20_PAUSE => erc20_pause_handler(sdk, input),
        SIG_ERC20_UNPAUSE => erc20_unpause_handler(sdk, input),
        SIG_ERC20_PERMIT => erc20_permit_handler(sdk, input),
        SIG_ERC20_NONCES => erc20_nonces_handler(sdk, input),
        SIG_ERC20_DOMAIN_SEPARATOR => erc20_domain_separator_handler(sdk, input),
        _ => erc20_unknown_method(sdk, input),
    }?;
    if evm_exit_code != 0 {
        write_evm_exit_message(evm_exit_code, |slice| sdk.write(slice));
        return Err(ExitCode::Panic);
    }
    Ok(())
}

system_entrypoint!(main_entry, deploy_entry);
