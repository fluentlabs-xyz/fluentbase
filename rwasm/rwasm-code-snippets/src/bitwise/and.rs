use crate::common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT};

#[no_mangle]
pub fn bitwise_and() {
    let mut a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    for i in 0..b.len() {
        a[i] &= b[i];
    }

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, a);
}
