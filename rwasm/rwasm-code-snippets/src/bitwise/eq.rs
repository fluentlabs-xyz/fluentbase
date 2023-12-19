use crate::common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT};

#[no_mangle]
fn bitwise_eq(// b0: u64,
    // b1: u64,
    // b2: u64,
    // b3: u64,
    // a0: u64,
    // a1: u64,
    // a2: u64,
    // a3: u64,
) /* -> (u64, u64, u64, u64) */
{
    // let s0;
    // if a0 == b0 && a1 == b1 && a2 == b2 && a3 == b3 {
    //     s0 = 1;
    // } else {
    //     s0 = 0;
    // }
    let mut is = true;
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let mut a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    for i in 0..a.len() {
        if is && a[i] != b[i] {
            is = false;
        };
        a[i] = 0;
    }
    a[a.len() - 1] = is as u8;
    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, a);

    // return (s0, 0, 0, 0);
}
