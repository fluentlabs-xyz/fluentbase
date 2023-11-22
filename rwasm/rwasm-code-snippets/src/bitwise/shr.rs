use crate::consts::BYTE_MAX_VAL;

#[no_mangle]
fn bitwise_shr(
    shift0: u64,
    shift1: u64,
    shift2: u64,
    shift3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    if shift0 != 0 || shift1 != 0 || shift2 != 0 || shift3 > BYTE_MAX_VAL {
        return (0, 0, 0, 0);
    }

    if shift3 >= 192 {
        let shift = shift3 - 192;
        let s3 = b0 >> shift;
        return (0, 0, 0, s3);
    }
    if shift3 >= 128 {
        let shift = shift3 - 128;
        let shift_inv = 64 - shift;
        let s2 = b0 >> shift;
        let s3 = b0 << shift_inv | b1 >> shift;
        return (0, 0, s2, s3);
    }
    if shift3 >= 64 {
        let shift = shift3 - 64;
        let shift_inv = 64 - shift;
        let s1 = b0 >> shift;
        let s2 = b0 << shift_inv | b1 >> shift;
        let s3 = b1 << shift_inv | b2 >> shift;
        return (0, s1, s2, s3);
    }

    let shift = shift3;
    let shift_inv = 64 - shift;
    let s0 = b0 >> shift;
    let s1 = b0 << shift_inv | b1 >> shift;
    let s2 = b1 << shift_inv | b2 >> shift;
    let s3 = b2 << shift_inv | b3 >> shift;
    return (s0, s1, s2, s3);
}
