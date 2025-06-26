use crate::{
    consts::{
        ERR_ALREADY_PAUSED,
        ERR_ALREADY_UNPAUSED,
        ERR_DECODE,
        ERR_INDEX_OUT_OF_BOUNDS,
        ERR_INSUFFICIENT_BALANCE,
        ERR_INVALID_META_NAME,
        ERR_INVALID_META_SYMBOL,
        ERR_INVALID_MINTER,
        ERR_INVALID_PAUSER,
        ERR_INVALID_RECIPIENT,
        ERR_MALFORMED_INPUT,
        ERR_MINTABLE_PLUGIN_NOT_ACTIVE,
        ERR_OVERFLOW,
        ERR_PAUSABLE_PLUGIN_NOT_ACTIVE,
        ERR_UNINIT,
        ERR_VALIDATION,
        SIG_ALLOWANCE,
        SIG_APPROVE,
        SIG_BALANCE_OF,
        SIG_DECIMALS,
        SIG_MINT,
        SIG_NAME,
        SIG_PAUSE,
        SIG_SYMBOL,
        SIG_TOTAL_SUPPLY,
        SIG_TRANSFER,
        SIG_TRANSFER_FROM,
        SIG_UNPAUSE,
    },
    storage::U256_LEN_BYTES,
};
use fluentbase_sdk::{HashSet, U256};

#[test]
fn u256_bytes_size() {
    assert_eq!(size_of::<U256>(), U256_LEN_BYTES);
}

#[test]
fn check_for_collisions() {
    let values = [
        ERR_MALFORMED_INPUT,
        ERR_INSUFFICIENT_BALANCE,
        ERR_INDEX_OUT_OF_BOUNDS,
        ERR_DECODE,
        ERR_INVALID_META_NAME,
        ERR_INVALID_META_SYMBOL,
        ERR_MINTABLE_PLUGIN_NOT_ACTIVE,
        ERR_PAUSABLE_PLUGIN_NOT_ACTIVE,
        ERR_ALREADY_PAUSED,
        ERR_ALREADY_UNPAUSED,
        ERR_INVALID_MINTER,
        ERR_INVALID_PAUSER,
        ERR_INVALID_RECIPIENT,
        ERR_OVERFLOW,
        ERR_VALIDATION,
        ERR_UNINIT,
        SIG_SYMBOL,
        SIG_NAME,
        SIG_DECIMALS,
        SIG_TOTAL_SUPPLY,
        SIG_BALANCE_OF,
        SIG_TRANSFER,
        SIG_TRANSFER_FROM,
        SIG_ALLOWANCE,
        SIG_APPROVE,
        SIG_MINT,
        SIG_PAUSE,
        SIG_UNPAUSE,
    ];
    let values_hashset: HashSet<u32> = values.into();
    assert_eq!(values.len(), values_hashset.len());
}
