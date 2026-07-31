//! Shared call validation, ABI, revert, and ERC-20 helpers.

use crate::{
    consts::*,
    storage::{chain_config_storage, initializer_storage},
};
use alloc::vec::Vec;
use fluentbase_sdk::{
    byteorder::BE,
    bytes::BytesMut,
    codec::{Encoder, FunctionArgs, SolidityABI},
    Address, ContextReader, ExitCode, SharedAPI, GENESIS_GOVERNANCE, U256,
};

pub(crate) fn revert<SDK: SharedAPI>(sdk: &mut SDK, code: u32) -> Result<(), ExitCode> {
    sdk.write(code.to_be_bytes());
    Err(ExitCode::Panic)
}

pub(crate) fn revert_with<SDK, T>(sdk: &mut SDK, code: u32, value: &T) -> Result<(), ExitCode>
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

pub(crate) fn safe_transfer<SDK: SharedAPI>(
    sdk: &mut SDK,
    recipient: Address,
    amount: U256,
) -> Result<(), ExitCode> {
    if amount.is_zero() {
        return Ok(());
    }
    let token = chain_config_storage()
        .staking_token_accessor()
        .get_checked(sdk)?;
    if token.is_zero() {
        return revert(sdk, ERR_ZERO_STAKING_TOKEN);
    }
    let input = erc20_transfer_input(recipient, amount)?;
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
