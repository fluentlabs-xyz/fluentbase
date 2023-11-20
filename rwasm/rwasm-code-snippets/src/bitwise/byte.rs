#[no_mangle]
fn bitwise_byte(
    a0: i64,
    a1: i64,
    a2: i64,
    a3: i64,
    b0: i64,
    b1: i64,
    b2: i64,
    b3: i64,
) -> (i64, i64, i64, i64) {
    const BYTE_MAX_VAL: i64 = 255;
    if a0 != 0 || a1 != 0 || a2 != 0 || a3 > 31 {
        return (0, 0, 0, 0);
    }

    let byte_value;
    if a3 >= 24 {
        let shift = (a3 - 24) * 8;
        byte_value = b0 & (BYTE_MAX_VAL << shift);
    } else if a3 >= 16 {
        let shift = (a3 - 16) * 8;
        byte_value = b0 & (BYTE_MAX_VAL << shift);
    } else if a3 >= 8 {
        let shift = (a3 - 8) * 8;
        byte_value = b0 & (BYTE_MAX_VAL << shift);
    } else {
        let shift = a3 * 8;
        byte_value = b3 & (BYTE_MAX_VAL << shift)
    }
    (0, 0, 0, byte_value)
}
