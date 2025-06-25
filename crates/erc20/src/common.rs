use crate::{
    consts::{ERR_CONVERSION, ERR_INVALID_MINTER, SIG_TOTAL_SUPPLY},
    storage::{ADDRESS_LEN_BYTES, U256_LEN_BYTES},
};
use fluentbase_sdk::{Address, SharedAPI, U256};

const BE_ENDIANNESS: bool = true;

#[inline(always)]
pub fn u256_from_bytes_slice(sdk: &mut impl SharedAPI, value: &[u8]) -> U256 {
    if let Some(v) = u256_from_bytes_slice_try(value) {
        return v;
    }
    sdk.evm_exit(ERR_CONVERSION);
}

#[inline(always)]
pub fn u256_from_bytes_slice_try(value: &[u8]) -> Option<U256> {
    if BE_ENDIANNESS {
        U256::try_from_be_slice(value)
    } else {
        U256::try_from_le_slice(value)
    }
}

#[inline(always)]
pub fn u256_from_fixed_bytes(sdk: &mut impl SharedAPI, value: &[u8; U256_LEN_BYTES]) -> U256 {
    u256_from_bytes_slice(sdk, value)
}
#[inline(always)]
pub fn fixed_bytes_from_u256(value: &U256) -> [u8; U256_LEN_BYTES] {
    if BE_ENDIANNESS {
        value.to_be_bytes::<U256_LEN_BYTES>()
    } else {
        value.to_le_bytes::<U256_LEN_BYTES>()
    }
}

#[inline(always)]
pub fn u256_from_address(sdk: &mut impl SharedAPI, value: &Address) -> U256 {
    u256_from_bytes_slice(sdk, value.as_slice())
}

#[inline(always)]
pub fn u256_from_address_try(value: &Address) -> Option<U256> {
    u256_from_bytes_slice_try(value.as_slice())
}
#[inline(always)]
pub fn address_from_u256(value: &U256) -> Address {
    Address::from_slice(&fixed_bytes_from_u256(value)[U256_LEN_BYTES - ADDRESS_LEN_BYTES..])
}
#[inline(always)]
pub fn sig_to_bytes(value: u32) -> [u8; size_of::<u32>()] {
    value.to_le_bytes()
}
