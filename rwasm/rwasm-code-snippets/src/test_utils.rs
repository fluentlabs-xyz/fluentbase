use ethereum_types::U256;

pub(crate) fn u256_into_le_tuple(u256: U256) -> (u64, u64, u64, u64) {
    (u256.0[3], u256.0[2], u256.0[1], u256.0[0])
}

pub(crate) fn u256_into_be_tuple(u256: U256) -> (u64, u64, u64, u64) {
    (u256.0[0], u256.0[1], u256.0[2], u256.0[3])
}

pub(crate) fn u256_from_le_u64(u64_0: u64, u64_1: u64, u64_2: u64, u64_3: u64) -> U256 {
    U256([u64_0, u64_1, u64_2, u64_3])
}

pub(crate) fn u256_from_be_u64(u64_0: u64, u64_1: u64, u64_2: u64, u64_3: u64) -> U256 {
    U256([u64_3, u64_2, u64_1, u64_0])
}

pub(crate) fn u256_from_le_tuple(a: &(u64, u64, u64, u64)) -> U256 {
    u256_from_le_u64(a.3, a.2, a.1, a.0)
}

pub(crate) fn u256_from_be_tuple(a: &(u64, u64, u64, u64)) -> U256 {
    u256_from_be_u64(a.0, a.1, a.2, a.3)
}
