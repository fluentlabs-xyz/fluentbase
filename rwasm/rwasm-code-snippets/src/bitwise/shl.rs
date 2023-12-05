use crate::consts::BYTE_MAX_VAL;

#[no_mangle]
fn bitwise_shl(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    if a3 != 0 || a2 != 0 || a1 != 0 || a0 > BYTE_MAX_VAL {
        // return (s0, s1, s2, s3);
    } else if a0 >= 192 {
        let shift = a0 - 192;
        s3 = b0 << shift;
        // return (s0, 0, 0, 0);
    } else if a0 >= 128 {
        let shift = a0 - 128;
        let shift_inv = 64 - shift;
        s2 = b0 << shift;
        s3 = b1 << shift | b0 >> shift_inv;
        // return (s0, s1, 0, 0);
    } else if a0 >= 64 {
        let shift = a0 - 64;
        let shift_inv = 64 - shift;
        s1 = b0 << shift;
        s2 = b1 << shift | b0 >> shift_inv;
        s3 = b2 << shift | b1 >> shift_inv;
        // return (s0, s1, s2, 0);
    } else {
        let shift = a0;
        let shift_inv = 64 - shift;
        s0 = b0 << shift;
        s1 = b1 << shift | b0 >> shift_inv;
        s2 = b2 << shift | b1 >> shift_inv;
        s3 = b3 << shift | b2 >> shift_inv;
    }

    (s0, s1, s2, s3)
}
