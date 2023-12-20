use crate::{
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
    consts::{U256_BYTES_COUNT, U8_MSBIT_IS_1},
};

#[no_mangle]
fn bitwise_sgt(// b0: u64,
    // b1: u64,
    // b2: u64,
    // b3: u64,
    // a0: u64,
    // a1: u64,
    // a2: u64,
    // a3: u64,
) /* -> (u64, u64, u64, u64) */
{
    // let a_sign = a0 & U64_MSBIT_IS_1;
    // let b_sign = b0 & U64_MSBIT_IS_1;
    // let mut r = (0, 0, 0, 0);
    //
    // if a_sign > b_sign {
    //     return (0, 0, 0, 0);
    // } else if a_sign < b_sign {
    //     r.0 = 1
    // } else {
    //     let a0_part = a0 - a_sign;
    //     let b0_part = b0 - b_sign;
    //     if a0_part > b0_part || a1 > b1 || a2 > b2 || a3 > b3 {
    //         r.0 = 1
    //     }
    // }
    // r
    let mut r = [0u8; U256_BYTES_COUNT as usize];
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let mut a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let a_sign = a[0] & U8_MSBIT_IS_1;
    let b_sign = b[0] & U8_MSBIT_IS_1;
    if a_sign > b_sign {
    } else if a_sign < b_sign {
        r[r.len() - 1] = 1;
    } else {
        let a0_part = a[0] - a_sign;
        let b0_part = b[0] - b_sign;
        if a0_part != b0_part {
            if a0_part > b0_part {
                r[r.len() - 1] = 1;
            };
        } else {
            for i in 1..a.len() {
                if a[i] != b[i] {
                    if a[i] > b[i] {
                        r[r.len() - 1] = 1;
                    };
                    break;
                }
            }
        }
    }

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, r);
}
