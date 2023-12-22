use crate::common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT};

#[no_mangle]
fn bitwise_or(// b0: u64,
    // b1: u64,
    // b2: u64,
    // b3: u64,
    // a0: u64,
    // a1: u64,
    // a2: u64,
    // a3: u64,
) /* -> (u64, u64, u64, u64) */
{
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    for i in 0..a.len() {
        a[i] |= b[i];
    }
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, a);
}
