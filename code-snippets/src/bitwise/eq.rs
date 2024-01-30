use crate::common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT};

#[no_mangle]
fn bitwise_eq() {
    let mut a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let mut is = true;
    for i in 0..a.len() {
        if is && a[i] != b[i] {
            is = false;
        };
        a[i] = 0;
    }
    a[a.len() - 1] = is as u8;
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, a);
}
