use crate::consts::{
    BITS_IN_BYTE,
    BYTE_SIGN_BIT_MASK,
    U64_ALL_BITS_ARE_0,
    U64_ALL_BITS_ARE_1,
    U64_HALF_BITS_COUNT,
    U64_LSBYTE_MASK,
};

#[no_mangle]
pub fn arithmetic_signextend(
    value0: u64,
    value1: u64,
    value2: u64,
    value3: u64,
    size0: u64,
    size1: u64,
    size2: u64,
    size3: u64,
) -> (u64, u64, u64, u64) {
    let mut s0: u64 = value0;
    let mut s1: u64 = value1;
    let mut s2: u64 = value2;
    let mut s3: u64 = value3;

    if size0 < U64_HALF_BITS_COUNT && size1 == 0 && size2 == 0 && size3 == 0 {
        let mut byte_value: u8 = 0;
        if size0 < 8 {
            let shift = size0 * BITS_IN_BYTE;
            byte_value = ((value0 >> shift) & U64_LSBYTE_MASK) as u8;
            if byte_value & BYTE_SIGN_BIT_MASK as u8 > 0 {
                let shift = shift + BITS_IN_BYTE;
                // fill with FF
                s0 = (U64_ALL_BITS_ARE_1 << shift) | value0;
                s1 = U64_ALL_BITS_ARE_1;
                s2 = U64_ALL_BITS_ARE_1;
                s3 = U64_ALL_BITS_ARE_1;
            } else {
                let shift = (7 - size0) * BITS_IN_BYTE;
                // fill with 00
                s0 = (U64_ALL_BITS_ARE_1 >> shift) & value0;
                s1 = U64_ALL_BITS_ARE_0;
                s2 = U64_ALL_BITS_ARE_0;
                s3 = U64_ALL_BITS_ARE_0;
            }
        } else if size0 < 16 {
            let shift = (size0 - 8) * BITS_IN_BYTE;
            byte_value = ((value1 >> shift) & U64_LSBYTE_MASK) as u8;
            if byte_value & BYTE_SIGN_BIT_MASK as u8 > 0 {
                let shift = shift + BITS_IN_BYTE;
                // fill with FF
                s1 = (U64_ALL_BITS_ARE_1 << shift) | value1;
                s2 = U64_ALL_BITS_ARE_1;
                s3 = U64_ALL_BITS_ARE_1;
            } else {
                let shift = (7 - size0) * BITS_IN_BYTE;
                // fill with 00
                s1 = (U64_ALL_BITS_ARE_1 >> shift) & value1;
                s2 = U64_ALL_BITS_ARE_0;
                s3 = U64_ALL_BITS_ARE_0;
            }
        } else if size0 < 24 {
            let shift = (size0 - 16) * BITS_IN_BYTE;
            byte_value = ((value2 >> shift) & U64_LSBYTE_MASK) as u8;
            if byte_value & BYTE_SIGN_BIT_MASK as u8 > 0 {
                let shift = shift + BITS_IN_BYTE;
                // fill with FF
                s2 = (U64_ALL_BITS_ARE_1 << shift) | value2;
                s3 = U64_ALL_BITS_ARE_1;
            } else {
                let shift = (7 - size0) * BITS_IN_BYTE;
                // fill with 00
                s2 = (U64_ALL_BITS_ARE_1 >> shift) & value2;
                s3 = U64_ALL_BITS_ARE_0;
            }
        } else {
            let shift = (size0 - 24) * BITS_IN_BYTE;
            byte_value = ((value3 >> shift) & U64_LSBYTE_MASK) as u8;
            if byte_value & BYTE_SIGN_BIT_MASK as u8 > 0 {
                let shift = shift + BITS_IN_BYTE;
                // fill with FF
                s3 = (U64_ALL_BITS_ARE_1 << shift) | value3;
            } else {
                let shift = (7 - size0) * BITS_IN_BYTE;
                // fill with 00
                s3 = (U64_ALL_BITS_ARE_1 >> shift) & value3;
            }
        }
    }

    (s0, s1, s2, s3)
}
