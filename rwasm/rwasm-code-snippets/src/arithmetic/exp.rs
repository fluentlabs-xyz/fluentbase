use crate::common::exp;

#[no_mangle]
pub fn arithmetic_exp(
    exp0: u64,
    exp1: u64,
    exp2: u64,
    exp3: u64,
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
) -> (u64, u64, u64, u64) {
    exp(v0, v1, v2, v3, exp0, exp1, exp2, exp3)
}
