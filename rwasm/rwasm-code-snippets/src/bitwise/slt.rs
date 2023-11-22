use crate::consts::U64_MSB_IS_1;

#[no_mangle]
fn bitwise_slt(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let a_sign = a0 & U64_MSB_IS_1;
    let b_sign = b0 & U64_MSB_IS_1;

    if a_sign < b_sign {
        return (0, 0, 0, 0);
    }
    if a_sign > b_sign {
        return (0, 0, 0, 1);
    }

    let a0_part = a0 - a_sign;
    let b0_part = b0 - b_sign;
    if a0_part < b0_part || a1 < b1 || a2 < b2 || a3 < b3 {
        return (0, 0, 0, 1);
    }
    return (0, 0, 0, 0);
}
