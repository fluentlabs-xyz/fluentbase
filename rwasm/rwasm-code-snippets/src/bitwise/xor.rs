use crate::common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT};

#[no_mangle]
fn bitwise_xor(// b0: u64,
    // b1: u64,
    // b2: u64,
    // b3: u64,
    // a0: u64,
    // a1: u64,
    // a2: u64,
    // a3: u64,
) /* -> (u64, u64, u64, u64) */
{
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let mut a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    for i in 0..a.len() {
        a[i] ^= b[i];
    }
    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, a);
}
