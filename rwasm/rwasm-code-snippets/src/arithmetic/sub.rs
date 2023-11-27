use crate::consts::{U64_MAX_VAL, U64_MSBIT_IS_1};

#[no_mangle]
fn arithmetic_sub(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let a0_sign = a0 & U64_MSBIT_IS_1;

    let mut borrow = 0;
    let mut s0 = 0;
    let mut s1 = 0;
    let mut s2 = 0;
    let mut s3 = 0;

    // if a3 >= b3 {
    //     s3 = a3 - b3;
    // } else {
    //     s3 = U64_MAX_VAL - b3 + a3 + 1;
    //     borrow = 1;
    // }
    //
    // if a2 >= b2 + borrow {
    //     s2 = a2 - b2 - borrow;
    //     borrow = 0;
    // } else {
    //     s2 = U64_MAX_VAL - b2 + a2;
    //     borrow = 1;
    // }
    //
    // if a1 >= b1 + borrow {
    //     s1 = a1 - b1 - borrow;
    //     borrow = 0;
    // } else {
    //     s1 = U64_MAX_VAL - b1 + a1;
    //     borrow = 1;
    // }
    //
    // if a0 >= b0 + borrow {
    //     s0 = a0 - b0 - borrow;
    //     // borrowed = 0;
    // } else {
    //     if a0_sign > 0 {
    //         s0 = U64_MAX_VAL - b0 + a0;
    //         // borrowed = 1;
    //     } else {
    //         // TODO process overflow
    //         s0 = U64_MSBIT_IS_1;
    //     }
    // }

    (U64_MAX_VAL, U64_MAX_VAL, U64_MAX_VAL, U64_MAX_VAL)
}
