use crate::common::shr;

#[no_mangle]
fn bitwise_shr(
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    shift0: u64,
    shift1: u64,
    shift2: u64,
    shift3: u64,
) -> (u64, u64, u64, u64) {
    shr(shift0, shift1, shift2, shift3, v0, v1, v2, v3)
    // let mut s0: u64 = 0;
    // let mut s1: u64 = 0;
    // let mut s2: u64 = 0;
    // let mut s3: u64 = 0;
    //
    // if shift3 != 0 || shift2 != 0 || shift1 != 0 || shift0 > BYTE_MAX_VAL {
    //     // return (0, 0, 0, 0);
    // } else if shift0 >= 192 {
    //     let shift = shift0 - 192;
    //     s0 = v3 >> shift;
    //     // return (0, 0, 0, s3);
    // } else if shift0 >= 128 {
    //     let shift = shift0 - 128;
    //     let shift_inv = 64 - shift;
    //     s1 = v3 >> shift;
    //     s0 = v3 << shift_inv | v2 >> shift;
    //     // return (0, 0, s2, s3);
    // } else if shift0 >= 64 {
    //     let shift = shift0 - 64;
    //     let shift_inv = 64 - shift;
    //     s2 = v3 >> shift;
    //     s1 = v3 << shift_inv | v2 >> shift;
    //     s0 = v2 << shift_inv | v1 >> shift;
    //     // return (0, s1, s2, s3);
    // } else {
    //     let shift = shift0;
    //     let shift_inv = 64 - shift;
    //     s3 = v3 >> shift;
    //     s2 = v3 << shift_inv | v2 >> shift;
    //     s1 = v2 << shift_inv | v1 >> shift;
    //     s0 = v1 << shift_inv | v0 >> shift;
    // }
    //
    // (s0, s1, s2, s3)
}
