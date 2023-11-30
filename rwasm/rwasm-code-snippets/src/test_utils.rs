use ethereum_types::U256;

pub(crate) fn u256_split_le(u256: U256) -> (u64, u64, u64, u64) {
    (u256.0[3], u256.0[2], u256.0[1], u256.0[0])
}

pub(crate) fn u256_split_be(u256: U256) -> (u64, u64, u64, u64) {
    (u256.0[0], u256.0[1], u256.0[2], u256.0[3])
}

pub(crate) fn combine_u64(u64_0: u64, u64_1: u64, u64_2: u64, u64_3: u64) -> U256 {
    U256([u64_0, u64_1, u64_2, u64_3])
}

pub(crate) fn u256_from_u64_be(u64_0: u64, u64_1: u64, u64_2: u64, u64_3: u64) -> U256 {
    U256([u64_3, u64_2, u64_1, u64_0])
}

pub(crate) fn split256(value: U256) -> (u64, u64, u64, u64) {
    (value.0[0], value.0[1], value.0[2], value.0[3])
}

pub(crate) fn combine256(a: u64, b: u64, c: u64, d: u64) -> U256 {
    U256([a, b, c, d])
}

pub(crate) fn u256_from_tuple_be(a: &(u64, u64, u64, u64)) -> U256 {
    u256_from_u64_be(a.0, a.1, a.2, a.3)
}
