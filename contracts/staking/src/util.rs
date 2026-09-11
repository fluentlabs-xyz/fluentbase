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

/// Decode a STATIC argument tuple, and refuse anything the declaration does not
/// account for.
///
/// K-13: `SolidityABI::decode` accepts a buffer that is LONGER than the type it
/// decodes, and it silently truncates an integer word to the declared width.
/// Measured on this contract before this wrapper existed: 32 bytes of tail after
/// a well-formed `isValidator(address)` decoded and dispatched happily, and
/// `setActiveValidatorsLength(uint32)` handed a word carrying bit 32 set and the
/// low bytes `7` wrote a cap of 7 — its `MAX_COMMITTEE_SIZE` ceiling never saw
/// the number the caller actually sent. A short buffer was already refused.
///
/// The check is a ROUND TRIP rather than a length comparison, because one test
/// catches both: re-encoding the decoded value reproduces the canonical bytes,
/// so a tail makes the lengths differ and a truncated integer makes the high
/// bytes of its word differ. It needs no per-field knowledge, which is what a
/// width check would need and what this layer does not have.
///
/// Only static types. A dynamic tuple has legal encodings that differ in their
/// offset layout, so byte equality would refuse calldata that is correct — see
/// [`decode_args`], which is the dynamic path and is deliberately left as it was.
///
/// This is the CONTRACT side of K-13 and not a fix to the decoder. Making
/// `SolidityABI::decode` itself strict would reach three other consumers, one of
/// which decodes two versioned storage payloads by trying both and taking the one
/// that parses — see the journal `.dpos-study/history/E1-CONTRACT-2.md` §6.
pub(crate) fn decode<T>(input: &[u8]) -> Result<T, ExitCode>
where
    T: Encoder<BE, 32, true, false>,
{
    let value: T =
        SolidityABI::<T>::decode(&input, 0).map_err(|_| ExitCode::MalformedBuiltinParams)?;
    if !<T as Encoder<BE, 32, true, false>>::IS_DYNAMIC {
        let mut canonical = BytesMut::new();
        SolidityABI::<T>::encode(&value, &mut canonical, 0)
            .map_err(|_| ExitCode::MalformedBuiltinParams)?;
        if canonical.as_ref() != input {
            return Err(ExitCode::MalformedBuiltinParams);
        }
    }
    Ok(value)
}

/// Decode a Solidity function's parameter tuple.
///
/// Dynamic function arguments omit the outer tuple offset used when a
/// dynamic Rust struct is encoded as a standalone ABI value.
///
/// NOT round-trip checked, unlike [`decode`]: every caller of this one passes a
/// dynamic tuple, and a dynamic tuple has legal encodings that differ in their
/// offset layout, so byte equality would refuse correct calldata. The tail and
/// the truncated-integer holes K-13 names therefore remain open on this path —
/// `initialize` and the three evidence routes. Both are bounded by what those
/// handlers then check: `initialize` validates every scalar it decodes and is
/// one-shot, and the evidence routes verify a BLS signature over the bytes.
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

/// Encode a Solidity function's return tuple without an outer tuple offset.
///
/// Beside [`write_abi`] rather than inside `consensus.rs`, where it used to live:
/// it is an ABI helper with no consensus in it, and `config.rs` needs it too.
pub(crate) fn write_returns<SDK, T>(sdk: &mut SDK, value: &T) -> Result<(), ExitCode>
where
    SDK: SharedAPI,
    T: FunctionArgs<BE, 32, true, false>,
{
    let mut output = BytesMut::new();
    SolidityABI::<T>::encode_function_args(value, &mut output)
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

/// Moves `amount` of the staking token from `from` to `to` under this
/// contract's allowance.
///
/// The recipient is a parameter and not this contract: a deposit names the
/// contract, and a stipend claim names the person, because the stipend is drawn
/// off the BLEND reserve and never lands here on its way.
pub(crate) fn safe_transfer_from<SDK: SharedAPI>(
    sdk: &mut SDK,
    from: Address,
    to: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    let token = chain_config_storage()
        .staking_token_accessor()
        .get_checked(sdk)?;
    if token.is_zero() {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    }
    let input = erc20_transfer_from_input(from, to, amount)?;
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
/// A refusal is a VALUE here, not a revert — but that is about who decides, not
/// about what gets decided. The only caller today (`seize_self_stake`) turns
/// every refusal into `ERR_STAKING_TOKEN_CALL_FAILED` and rolls its whole
/// penalty back, which is the opposite of what this function's earlier doc
/// promised and is deliberate (K-22). What this still buys is that the CALLER
/// chooses: all three refusal vectors — a reverting call, the `false` a plain
/// ERC-20 returns, and a return this cannot decode — arrive here as `false`
/// rather than as a `?` that would unwind before the caller saw them.
///
/// Unlike `safe_transfer` this never forwards the callee's revert data into the
/// output buffer, so a caller that DOES revert writes its own error selector
/// into a buffer the callee has not already claimed.
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
    // `safe_transfer`, and chosen rather than inherited. Garbage from the token
    // reads as "the tokens did not move" and folds like any other refusal, so
    // the caller sees one outcome to decide about instead of three.
    Ok(result.data.is_empty() || SolidityABI::<bool>::decode(&result.data, 0).unwrap_or(false))
}

fn erc20_scalar_read<SDK: SharedAPI>(sdk: &mut SDK, token: Address, input: Vec<u8>) -> U256 {
    let result = sdk.static_call(token, &input, None);
    if !result.status.is_ok() {
        return U256::ZERO;
    }
    SolidityABI::<U256>::decode(&result.data, 0).unwrap_or(U256::ZERO)
}

/// BLEND the reserve can actually deliver right now: `min(balance, allowance)`.
///
/// Every failure of the CALLEE reads as ZERO — a missing token, a reverting
/// call, a static call that ran out of fuel, a return this cannot decode. That
/// rule is what lets this be called from `close_epoch`, which is a
/// pre-execution system call: a propagated failure there is a block-execution
/// failure on every node and no transaction can repair it. It was confirmed on
/// the real rWasm blob before it was relied on — a nested failure, by revert AND
/// by fuel exhaustion, discards only its own frame and the outer one carries on.
///
/// It is NOT unconditionally infallible, and the difference is worth stating.
/// Reading this contract's own token slot and encoding two fixed-size arguments
/// can still return `Err`. Both are host-level failures on data this chain
/// owns, in the same class as every other storage read in the close; the
/// third-party token is the input nobody here controls, and that one is what is
/// swallowed.
///
/// Both halves are needed and neither implies the other: a funded reserve that
/// revoked its approval can pay nothing, and a generous approval over an empty
/// balance is a promise the token will refuse.
pub(crate) fn reserve_available<SDK: SharedAPI>(
    sdk: &mut SDK,
    reserve: Address,
) -> Result<U256, ExitCode> {
    let token = chain_config_storage()
        .staking_token_accessor()
        .get_checked(sdk)?;
    if token.is_zero() {
        return Ok(U256::ZERO);
    }
    let spender = sdk.context().contract_address();

    let mut params = BytesMut::new();
    SolidityABI::<Address>::encode(&reserve, &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = SIG_ERC20_BALANCE_OF.to_be_bytes().to_vec();
    input.extend_from_slice(&params);
    let balance = erc20_scalar_read(sdk, token, input);
    if balance.is_zero() {
        return Ok(U256::ZERO);
    }

    let mut params = BytesMut::new();
    SolidityABI::<(Address, Address)>::encode(&(reserve, spender), &mut params, 0)
        .map_err(|_| ExitCode::MalformedBuiltinParams)?;
    let mut input = SIG_ERC20_ALLOWANCE.to_be_bytes().to_vec();
    input.extend_from_slice(&params);
    let allowance = erc20_scalar_read(sdk, token, input);

    Ok(core::cmp::min(balance, allowance))
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
