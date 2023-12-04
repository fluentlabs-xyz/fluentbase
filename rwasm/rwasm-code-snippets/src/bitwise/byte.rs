use crate::consts::{BITS_IN_BYTE, BYTE_MAX_VAL, U64_BYTES_COUNT, U64_LSBYTE_MASK};

#[no_mangle]
fn bitwise_byte(
    shift0: u64,
    shift1: u64,
    shift2: u64,
    shift3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    if shift3 != 0 || shift2 != 0 || shift1 != 0 || shift0 > 31 {
    } else if shift0 >= 24 {
        let shift = ((U64_BYTES_COUNT - 1) - shift0 - 24) * BITS_IN_BYTE;
        s0 = (b0 >> shift & U64_LSBYTE_MASK);
    } else if shift0 >= 16 {
        let shift = ((U64_BYTES_COUNT - 1) - shift0 - 16) * BITS_IN_BYTE;
        s0 = (b1 >> shift & U64_LSBYTE_MASK);
    } else if shift0 >= 8 {
        let shift = ((U64_BYTES_COUNT - 1) - shift0 - 8) * BITS_IN_BYTE;
        s0 = (b2 >> shift & U64_LSBYTE_MASK);
    } else {
        let shift = ((U64_BYTES_COUNT - 1) - shift0) * BITS_IN_BYTE;
        s0 = (b3 >> shift & U64_LSBYTE_MASK);
    }

    (s0, s1, s2, s3)
}
