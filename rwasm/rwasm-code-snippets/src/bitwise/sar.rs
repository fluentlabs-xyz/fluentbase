use crate::consts::{BYTE_MAX_VAL, U64_ALL_BITS_ARE_1, U64_MSBIT_IS_1};

#[no_mangle]
fn bitwise_sar(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    let mut r = (0, 0, 0, 0);
    let b0_sign = b3 & U64_MSBIT_IS_1;

    if a3 != 0 || a2 != 0 || a1 != 0 || a0 > BYTE_MAX_VAL {
        if b0_sign > 0 {
            r = (
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
            );
        }
    } else if a0 >= 192 {
        let shift = a0 - 192;
        let shift_inv = 64 - shift;
        r.0 = b3 >> shift;
        if b0_sign > 0 {
            r.0 = r.0 | U64_ALL_BITS_ARE_1 << shift_inv;
            return (
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                r.0,
            );
        }
    } else if a0 >= 128 {
        let shift = a0 - 128;
        let shift_inv = 64 - shift;
        r.1 = b3 >> shift;
        r.0 = b3 << shift_inv | b2 >> shift;
        if b0_sign > 0 {
            r.1 = r.1 | U64_ALL_BITS_ARE_1 << shift_inv;
            return (U64_ALL_BITS_ARE_1, U64_ALL_BITS_ARE_1, r.1, r.0);
        }
    } else if a0 >= 64 {
        let shift = a0 - 64;
        let shift_inv = 64 - shift;
        r.2 = b3 >> shift;
        r.1 = b3 << shift_inv | b2 >> shift;
        r.0 = b2 << shift_inv | b1 >> shift;
        if b0_sign > 0 {
            r.2 = r.2 | U64_ALL_BITS_ARE_1 << shift_inv;
            return (U64_ALL_BITS_ARE_1, r.2, r.1, r.0);
        }
    } else {
        let shift = a0;
        let shift_inv = 64 - shift;
        r.3 = b3 >> shift;
        r.2 = b3 << shift_inv | b2 >> shift;
        r.1 = b2 << shift_inv | b1 >> shift;
        r.0 = b1 << shift_inv | b0 >> shift;
        if b0_sign > 0 {
            r.3 = r.3 | U64_ALL_BITS_ARE_1 << shift_inv;
        }
    }
    r
}
