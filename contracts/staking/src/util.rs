//! Shared call validation, ABI, revert, and ERC-20 helpers.

use crate::{
    consts::*,
    math,
    storage::{chain_config_storage, initializer_storage},
};
use alloc::vec::Vec;
use fluentbase_sdk::{
    byteorder::BE,
    bytes::BytesMut,
    codec::{Encoder, FunctionArgs, SolidityABI},
    Address, Bytes, ContextReader, ExitCode, SharedAPI, SyscallResult, GENESIS_GOVERNANCE, U256,
};

pub(crate) fn revert<SDK: SharedAPI, T>(sdk: &mut SDK, code: u32) -> Result<T, ExitCode> {
    sdk.write(code.to_be_bytes());
    Err(ExitCode::Panic)
}

pub(crate) fn revert_with<SDK, T, R>(sdk: &mut SDK, code: u32, value: &T) -> Result<R, ExitCode>
where
    SDK: SharedAPI,
    T: Encoder<BE, 32, true, false>,
{
    let mut params = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut output = code.to_be_bytes().to_vec();
    output.extend_from_slice(&params);
    sdk.write(output);
    Err(ExitCode::Panic)
}

pub(crate) fn ensure_non_payable<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if !sdk.context().contract_value().is_zero() {
        return Err(ExitCode::Panic);
    }
    Ok(())
}

pub(crate) fn ensure_mutable<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if sdk.context().contract_is_static() {
        return Err(ExitCode::StateChangeDuringStaticCall);
    }
    Ok(())
}

pub(crate) fn ensure_initialized<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    if !initializer_storage()
        .initialized_accessor()
        .get_checked(sdk)?
    {
        return revert(sdk, ERR_NOT_INITIALIZED);
    }
    Ok(())
}

pub(crate) fn ensure_governance<SDK: SharedAPI>(sdk: &mut SDK) -> Result<(), ExitCode> {
    ensure_initialized(sdk)?;
    if sdk.context().contract_caller() != GENESIS_GOVERNANCE {
        return revert(sdk, ERR_ONLY_GOVERNANCE);
    }
    Ok(())
}

pub(crate) fn current_epoch<SDK: SharedAPI>(sdk: &SDK) -> Result<u64, ExitCode> {
    current_epoch_at_block(sdk, sdk.context().block_number())
}

pub(crate) fn next_epoch<SDK: SharedAPI>(sdk: &SDK) -> Result<u64, ExitCode> {
    current_epoch(sdk)?
        .checked_add(1)
        .ok_or(ExitCode::IntegerOverflow)
}

pub(crate) fn current_epoch_at_block<SDK: SharedAPI>(
    sdk: &SDK,
    block_number: u64,
) -> Result<u64, ExitCode> {
    let config = chain_config_storage();
    let activation = config.dpos_activation_block_accessor().get_checked(sdk)?;
    let interval = config.epoch_block_interval_accessor().get_checked(sdk)?;
    math::epoch_at_block(block_number, activation, interval).ok_or(ExitCode::IntegerDivisionByZero)
}

pub(crate) fn decode<T>(input: &[u8]) -> Result<T, ExitCode>
where
    T: Encoder<BE, 32, true, false>,
{
    SolidityABI::<T>::decode(&input, 0).map_err(|_| ExitCode::MalformedBuiltinParams)
}

/// Decode a Solidity function's parameter tuple.
///
/// Dynamic function arguments omit the outer tuple offset used when a
/// dynamic Rust struct is encoded as a standalone ABI value.
pub(crate) fn decode_args<T>(input: &[u8]) -> Result<T, ExitCode>
where
    T: FunctionArgs<BE, 32, true, false>,
{
    SolidityABI::<T>::decode_function_args(&input).map_err(|_| ExitCode::MalformedBuiltinParams)
}

pub(crate) fn write_abi<SDK, T>(sdk: &mut SDK, value: &T) -> Result<(), ExitCode>
where
    SDK: SharedAPI,
    T: Encoder<BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode(value, &mut output, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    sdk.write(output.freeze());
    Ok(())
}

fn erc20_transfer_from_input(
    from: Address,
    to: Address,
    amount: U256,
) -> Result<Vec<u8>, ExitCode> {
    let mut params = BytesMut::new();
    SolidityABI::<(Address, Address, U256)>::encode(&(from, to, amount), &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = SIG_ERC20_TRANSFER_FROM.to_be_bytes().to_vec();
    input.extend_from_slice(&params);
    Ok(input)
}

fn erc20_transfer_input(to: Address, amount: U256) -> Result<Vec<u8>, ExitCode> {
    let mut params = BytesMut::new();
    SolidityABI::<(Address, U256)>::encode(&(to, amount), &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = SIG_ERC20_TRANSFER.to_be_bytes().to_vec();
    input.extend_from_slice(&params);
    Ok(input)
}

pub(crate) fn safe_transfer_from<SDK: SharedAPI>(
    sdk: &mut SDK,
    from: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    let token = chain_config_storage()
        .staking_token_accessor()
        .get_checked(sdk)?;
    if token.is_zero() {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    }
    let recipient = sdk.context().contract_address();
    let input = erc20_transfer_from_input(from, recipient, amount)?;
    let result = sdk.call(token, U256::ZERO, &input, None);
    if !result.status.is_ok() {
        sdk.write(result.data);
        return Err(result.status);
    }
    if !result.data.is_empty()
        && !SolidityABI::<bool>::decode(&result.data, 0)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?
    {
        return revert(sdk, ERR_STAKING_TOKEN_CALL_FAILED);
    }
    Ok(())
}

/// Issues the ERC-20 transfer; `None` means no staking token is configured.
///
/// The calldata and the call have one definition so a change to either cannot
/// reach one caller and miss the other. How a revert or a `false` is *read* is
/// deliberately not shared: the two callers need opposite things from it, and
/// each says so where it decides.
fn erc20_transfer_call<SDK: SharedAPI>(
    sdk: &mut SDK,
    recipient: Address,
    amount: U256,
) -> Result<Option<SyscallResult<Bytes>>, ExitCode> {
    let token = chain_config_storage()
        .staking_token_accessor()
        .get_checked(sdk)?;
    if token.is_zero() {
        return Ok(None);
    }
    let input = erc20_transfer_input(recipient, amount)?;
    Ok(Some(sdk.call(token, U256::ZERO, &input, None)))
}

pub(crate) fn safe_transfer<SDK: SharedAPI>(
    sdk: &mut SDK,
    recipient: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    if amount.is_zero() {
        return Ok(());
    }
    let Some(result) = erc20_transfer_call(sdk, recipient, amount)? else {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    };
    if !result.status.is_ok() {
        sdk.write(result.data);
        return Err(result.status);
    }
    // A malformed return is an error here, not a refusal — the opposite of
    // `try_transfer`. This caller is allowed to revert, so it fails loud rather
    // than guessing what a token that answered garbage meant.
    if !result.data.is_empty()
        && !SolidityABI::<bool>::decode(&result.data, 0)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?
    {
        return revert(sdk, ERR_STAKING_TOKEN_CALL_FAILED);
    }
    Ok(())
}

/// Attempts an ERC-20 transfer and reports whether the tokens moved.
///
/// A refusal is a value here, not a revert: a caller whose earlier effects must
/// survive a hostile or misconfigured recipient decides for itself what to do
/// with the failure. Both refusal vectors count — a reverting call and the
/// `false` a plain ERC-20 returns — because either one reaching `?` would undo
/// the caller. Unlike `safe_transfer` this never forwards the callee's revert
/// data into the output buffer: the caller goes on to return `Ok`, and that
/// buffer is the transaction's return value.
pub(crate) fn try_transfer<SDK: SharedAPI>(
    sdk: &mut SDK,
    recipient: Address,
    amount: U256,
) -> Result<bool, ExitCode> {
    if amount.is_zero() {
        return Ok(true);
    }
    let Some(result) = erc20_transfer_call(sdk, recipient, amount)? else {
        return Ok(false);
    };
    if !result.status.is_ok() {
        return Ok(false);
    }
    // A malformed return counts as a refusal, not an error — the opposite of
    // `safe_transfer`, and chosen rather than inherited. This caller must not
    // revert, so garbage from the token reads as "the tokens did not move" and
    // folds like any other refusal.
    Ok(result.data.is_empty() || SolidityABI::<bool>::decode(&result.data, 0).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erc20_transfer_from_calldata_is_standard_abi() {
        let from = Address::with_last_byte(0x11);
        let to = Address::with_last_byte(0x22);
        let amount = U256::from(123);
        let input = erc20_transfer_from_input(from, to, amount).unwrap();
        assert_eq!(&input[..4], &SIG_ERC20_TRANSFER_FROM.to_be_bytes());
        assert_eq!(
            SolidityABI::<(Address, Address, U256)>::decode(&&input[4..], 0).unwrap(),
            (from, to, amount)
        );
    }
}
