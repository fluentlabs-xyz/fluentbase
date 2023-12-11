use crate::common::{add, mod_impl};

#[no_mangle]
pub fn arithmetic_addmod(
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
    let r = add(a0, a1, a2, a3, b0, b1, b2, b3);
    mod_impl(r.0, r.1, r.2, r.3, n0, n1, n2, n3)
}
