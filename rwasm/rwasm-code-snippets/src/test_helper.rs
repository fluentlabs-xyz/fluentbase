use ethereum_types::U256;

// #[derive(Debug, Copy, Clone)]
// pub(super) struct U256([u64; 4]);

pub(super) fn split_u256(u256: U256) -> (u64, u64, u64, u64) {
    let limb0 = u256.0[0];
    let limb1 = u256.0[1];
    let limb2 = u256.0[2];
    let limb3 = u256.0[3];

    (limb0, limb1, limb2, limb3)
}

pub(super) fn combine_u64(u64_0: u64, u64_1: u64, u64_2: u64, u64_3: u64) -> U256 {
    U256([u64_0, u64_1, u64_2, u64_3])
}

pub(super) fn split256(value: U256) -> (u64, u64, u64, u64) {
    (value.0[0], value.0[1], value.0[2], value.0[3])
}

pub(super) fn combine256(a: u64, b: u64, c: u64, d: u64) -> U256 {
    U256([a, b, c, d])
}
