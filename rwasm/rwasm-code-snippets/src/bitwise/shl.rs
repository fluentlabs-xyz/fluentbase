use crate::consts::BYTE_MAX_VAL;

#[no_mangle]
fn bitwise_shl(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    if a0 != 0 || a1 != 0 || a2 != 0 || a3 > BYTE_MAX_VAL {
        // return (s0, s1, s2, s3);
    } else if a3 >= 192 {
        let shift = a3 - 192;
        s0 = b3 << shift;
        // return (s0, 0, 0, 0);
    } else if a3 >= 128 {
        let shift = a3 - 128;
        let shift_inv = 64 - shift;
        s1 = b3 << shift;
        s0 = b2 << shift | b3 >> shift_inv;
        // return (s0, s1, 0, 0);
    } else if a3 >= 64 {
        let shift = a3 - 64;
        let shift_inv = 64 - shift;
        s2 = b3 << shift;
        s1 = b2 << shift | b3 >> shift_inv;
        s0 = b1 << shift | b2 >> shift_inv;
        // return (s0, s1, s2, 0);
    } else {
        let shift = a3;
        let shift_inv = 64 - shift;
        s3 = b3 << shift;
        s2 = b2 << shift | b3 >> shift_inv;
        s1 = b1 << shift | b2 >> shift_inv;
        s0 = b0 << shift | b1 >> shift_inv;
    }

    (s0, s1, s2, s3)
}
