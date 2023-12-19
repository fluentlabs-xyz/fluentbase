use crate::{
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};

#[no_mangle]
fn bitwise_lt(// b0: u64,
    // b1: u64,
    // b2: u64,
    // b3: u64,
    // a0: u64,
    // a1: u64,
    // a2: u64,
    // a3: u64,
) /* -> (u64, u64, u64, u64) */
{
    // let mut s0 = 0;
    // if a3 < b3 {
    //     s0 = 1;
    // } else if a3 > b3 {
    //     s0 = 0;
    // } else if a2 < b2 {
    //     s0 = 1;
    // } else if a2 > b2 {
    //     s0 = 0;
    // } else if a1 < b1 {
    //     s0 = 1;
    // } else if a1 > b1 {
    //     s0 = 0;
    // } else if a0 < b0 {
    //     s0 = 1;
    // }
    //
    // return (s0, 0, 0, 0);

    let mut r = [0u8; U256_BYTES_COUNT as usize];
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let mut a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    for i in 0..a.len() {
        if a[i] != b[i] {
            if a[i] < b[i] {
                r[r.len() - 1] = 1;
            };
            break;
        }
    }
    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, r);
}
