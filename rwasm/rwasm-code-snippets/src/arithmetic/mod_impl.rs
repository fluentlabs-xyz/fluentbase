use crate::{
    common::{mod_impl, try_divide_close_numbers},
    consts::U256_BYTES_COUNT,
};

#[no_mangle]
pub fn arithmetic_mod(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    mod_impl(a0, a1, a2, a3, b0, b1, b2, b3)
}
