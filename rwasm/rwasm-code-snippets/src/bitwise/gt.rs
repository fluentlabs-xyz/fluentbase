use crate::{
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};

#[no_mangle]
fn bitwise_gt() {
    let mut a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let mut r = [0u8; U256_BYTES_COUNT as usize];
    for i in 0..a.len() {
        if a[i] != b[i] {
            if a[i] > b[i] {
                r[r.len() - 1] = 1;
            };
            break;
        }
    }
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, r);
}
