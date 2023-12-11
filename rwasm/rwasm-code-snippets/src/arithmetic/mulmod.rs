use crate::common::{mod_impl, mul};

#[no_mangle]
pub fn arithmetic_mulmod(
    n0: u64,
    n1: u64,
    n2: u64,
    n3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    let r1 = mod_impl(a0, a1, a2, a3, n0, n1, n2, n3);
    let r2 = mod_impl(b0, b1, b2, b3, n0, n1, n2, n3);
    mul(r1.0, r1.1, r1.2, r1.3, r2.0, r2.1, r2.2, r2.3)
}
