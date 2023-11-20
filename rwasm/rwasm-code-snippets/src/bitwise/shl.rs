#[no_mangle]
fn bitwise_shl(
    a0: i64,
    a1: i64,
    a2: i64,
    a3: i64,
    b0: i64,
    b1: i64,
    b2: i64,
    b3: i64,
) -> (i64, i64, i64, i64) {
    if a0 != 0 || a1 != 0 || a2 != 0 || a3 > 255 {
        return (0, 0, 0, 0);
    }

    if a3 >= 192 {
        let shift = a3 - 192;
        let s0 = b3 << shift;
        return (s0, 0, 0, 0);
    }
    if a3 >= 128 {
        let shift = a3 - 128;
        let shift_inv = 64 - shift;
        let s1 = b3 << shift;
        let s0 = b2 << shift | b3 >> shift_inv;
        return (s0, s1, 0, 0);
    }
    if a3 >= 64 {
        let shift = a3 - 64;
        let shift_inv = 64 - shift;
        let s2 = b3 << shift;
        let s1 = b2 << shift | b3 >> shift_inv;
        let s0 = b1 << shift | b2 >> shift_inv;
        return (s0, s1, s2, 0);
    }

    let shift = a3;
    let shift_inv = 64 - shift;
    let s3 = b3 << shift;
    let s2 = b2 << shift | b3 >> shift_inv;
    let s1 = b1 << shift | b2 >> shift_inv;
    let s0 = b0 << shift | b1 >> shift_inv;
    return (s0, s1, s2, s3);
}
