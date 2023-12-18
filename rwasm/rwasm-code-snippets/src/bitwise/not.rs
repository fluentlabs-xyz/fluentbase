use crate::common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT};

#[no_mangle]
fn bitwise_not() {
    let mut a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    for i in 0..a.len() {
        a[i] = !a[i];
    }

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, a);
}
