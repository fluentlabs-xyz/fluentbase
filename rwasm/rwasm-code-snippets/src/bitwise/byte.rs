use crate::consts::{BITS_IN_BYTE, BYTE_MAX_VAL};

#[no_mangle]
fn bitwise_byte(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    if a0 != 0 || a1 != 0 || a2 != 0 || a3 > 31 {
        return (0, 0, 0, 0);
    }

    let byte_value;
    if a3 >= 24 {
        let shift = (a3 - 24) * BITS_IN_BYTE;
        byte_value = b0 & (BYTE_MAX_VAL << shift);
    } else if a3 >= 16 {
        let shift = (a3 - 16) * BITS_IN_BYTE;
        byte_value = b1 & (BYTE_MAX_VAL << shift);
    } else if a3 >= 8 {
        let shift = (a3 - 8) * BITS_IN_BYTE;
        byte_value = b2 & (BYTE_MAX_VAL << shift);
    } else {
        let shift = a3 * BITS_IN_BYTE;
        byte_value = b3 & (BYTE_MAX_VAL << shift)
    }
    (0, 0, 0, byte_value)
}
