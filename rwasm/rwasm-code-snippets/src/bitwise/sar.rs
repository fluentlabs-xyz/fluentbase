use crate::consts::{BYTE_MAX_VAL, U64_ALL_BITS_ARE_1, U64_MSB_IS_1};

#[no_mangle]
fn bitwise_sar(
    shift0: u64,
    shift1: u64,
    shift2: u64,
    shift3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let b0_sign = b0 & U64_MSB_IS_1;
    if shift0 != 0 || shift1 != 0 || shift2 != 0 || shift3 > BYTE_MAX_VAL {
        if b0_sign > 0 {
            return (
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
            );
        }
        return (0, 0, 0, 0);
    }

    if shift3 >= 192 {
        let shift = shift3 - 192;
        let shift_inv = 64 - shift;
        let s3 = b0 >> shift;
        if b0_sign > 0 {
            let s3 = s3 | U64_ALL_BITS_ARE_1 << shift_inv;
            return (
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                s3,
            );
        }
        return (0, 0, 0, s3);
    }
    if shift3 >= 128 {
        let shift = shift3 - 128;
        let shift_inv = 64 - shift;
        let s2 = b0 >> shift;
        let s3 = b0 << shift_inv | b1 >> shift;
        if b0_sign > 0 {
            let s2 = s2 | U64_ALL_BITS_ARE_1 << shift_inv;
            return (U64_ALL_BITS_ARE_1, U64_ALL_BITS_ARE_1, s2, s3);
        }
        return (0, 0, s2, s3);
    }
    if shift3 >= 64 {
        let shift = shift3 - 64;
        let shift_inv = 64 - shift;
        let s1 = b0 >> shift;
        let s2 = b0 << shift_inv | b1 >> shift;
        let s3 = b1 << shift_inv | b2 >> shift;
        if b0_sign > 0 {
            let s1 = s1 | U64_ALL_BITS_ARE_1 << shift_inv;
            return (U64_ALL_BITS_ARE_1, s1, s2, s3);
        }
        return (0, s1, s2, s3);
    }

    let shift = shift3;
    let shift_inv = 64 - shift;
    let s0 = b0 >> shift;
    let s1 = b0 << shift_inv | b1 >> shift;
    let s2 = b1 << shift_inv | b2 >> shift;
    let s3 = b2 << shift_inv | b3 >> shift;
    if b0_sign > 0 {
        let s0 = s0 | U64_ALL_BITS_ARE_1 << shift_inv;
        return (s0, s1, s2, s3);
    }
    return (s0, s1, s2, s3);
}
