use crate::{
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};

#[no_mangle]
fn bitwise_iszero() {
    let mut a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let mut r = [0u8; U256_BYTES_COUNT as usize];
    r[r.len() - 1] = 1;

    for i in 0..a.len() {
        if a[i] != 0 {
            r[r.len() - 1] = 0;
            break;
        }
    }

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, r);
}
