use crate::common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT};

#[no_mangle]
fn bitwise_xor() {
    let mut a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    for i in 0..a.len() {
        a[i] ^= b[i];
    }
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, a);
}
