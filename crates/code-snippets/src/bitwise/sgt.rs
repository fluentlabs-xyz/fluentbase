use crate::{
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::{U256_BYTES_COUNT, U8_MSBIT_IS_1},
};

#[no_mangle]
fn bitwise_sgt() {
    let mut a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let mut r = [0u8; U256_BYTES_COUNT as usize];
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

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, r);
}
