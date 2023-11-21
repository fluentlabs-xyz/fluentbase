#[no_mangle]
fn bitwise_shl(
    shift0: u64,
    shift1: u64,
    shift2: u64,
    shift3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    if shift0 != 0 || shift1 != 0 || shift2 != 0 || shift3 > 255 {
        return (0, 0, 0, 0);
    }

    if shift3 >= 192 {
        let shift = shift3 - 192;
        let s0 = b3 << shift;
        return (s0, 0, 0, 0);
    }
    if shift3 >= 128 {
        let shift = shift3 - 128;
        let shift_inv = 64 - shift;
        let s1 = b3 << shift;
        let s0 = b2 << shift | b3 >> shift_inv;
        return (s0, s1, 0, 0);
    }
    if shift3 >= 64 {
        let shift = shift3 - 64;
        let shift_inv = 64 - shift;
        let s2 = b3 << shift;
        let s1 = b2 << shift | b3 >> shift_inv;
        let s0 = b1 << shift | b2 >> shift_inv;
        return (s0, s1, s2, 0);
    }

    let shift = shift3;
    let shift_inv = 64 - shift;
    let s3 = b3 << shift;
    let s2 = b2 << shift | b3 >> shift_inv;
    let s1 = b1 << shift | b2 >> shift_inv;
    let s0 = b0 << shift | b1 >> shift_inv;
    return (s0, s1, s2, s3);
}
