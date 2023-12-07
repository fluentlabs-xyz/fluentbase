use crate::consts::{
    BITS_IN_BYTE,
    BYTE_SIGN_BIT_MASK,
    U64_ALL_BITS_ARE_0,
    U64_ALL_BITS_ARE_1,
    U64_BYTES_COUNT,
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
    let mut res = [value0, value1, value2, value3];

    if size0 < U64_HALF_BITS_COUNT && size1 == 0 && size2 == 0 && size3 == 0 {
        let mut byte_value: u8 = 0;
        let res_idx = size0 as usize / 8;
        let filler: u64;
        let shift = (size0 - res_idx as u64 * U64_BYTES_COUNT) * BITS_IN_BYTE;
        byte_value = ((res[res_idx] >> shift) & U64_LSBYTE_MASK) as u8;
        if byte_value >= BYTE_SIGN_BIT_MASK as u8 {
            let shift = shift + BITS_IN_BYTE;
            res[res_idx] = (U64_ALL_BITS_ARE_1 << shift) | res[res_idx];
            filler = U64_ALL_BITS_ARE_1;
        } else {
            let shift = (7 - size0) * BITS_IN_BYTE;
            res[res_idx] = (U64_ALL_BITS_ARE_1 >> shift) & res[res_idx];
            filler = U64_ALL_BITS_ARE_0;
        }
        for i in res_idx + 1..4 {
            res[i] = filler
        }
    }

    (res[0], res[1], res[2], res[3])
}
