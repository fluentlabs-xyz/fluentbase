use fluentbase_sdk::U256;

pub fn u256_try_from_slice(slice: &[u8]) -> Option<U256> {
    U256::try_from_le_slice(slice)
}

pub fn slice_from_u256(value: &U256) -> &[u8] {
    value.as_le_slice()
}
