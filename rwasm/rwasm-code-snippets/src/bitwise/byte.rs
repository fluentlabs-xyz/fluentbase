use crate::consts::{BITS_IN_BYTE, BYTES_IN_WASM_I64, BYTE_MAX_VAL, U64_LSBYTE_MASK};

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

    if shift0 != 0 || shift1 != 0 || shift2 != 0 || shift3 > 31 {
    } else if shift3 >= 24 {
        let shift = ((BYTES_IN_WASM_I64 - 1) - shift3 - 24) * BITS_IN_BYTE;
        s3 = (b3 >> shift & U64_LSBYTE_MASK);
    } else if shift3 >= 16 {
        let shift = ((BYTES_IN_WASM_I64 - 1) - shift3 - 16) * BITS_IN_BYTE;
        s3 = (b2 >> shift & U64_LSBYTE_MASK);
    } else if shift3 >= 8 {
        let shift = ((BYTES_IN_WASM_I64 - 1) - shift3 - 8) * BITS_IN_BYTE;
        s3 = (b1 >> shift & U64_LSBYTE_MASK);
    } else {
        let shift = ((BYTES_IN_WASM_I64 - 1) - shift3) * BITS_IN_BYTE;
        s3 = (b0 >> shift & U64_LSBYTE_MASK);
    }

    (s0, s1, s2, s3)
}
