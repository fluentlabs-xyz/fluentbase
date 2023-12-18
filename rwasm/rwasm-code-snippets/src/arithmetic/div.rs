use crate::common::div_le;

#[no_mangle]
pub fn arithmetic_div(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    div_le((a0, a1, a2, a3), (b0, b1, b2, b3))
}
