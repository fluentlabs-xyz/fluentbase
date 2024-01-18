use crate::{
    common::{u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::{
        BITS_IN_BYTE,
        BYTE_SIGN_BIT_MASK,
        U64_ALL_BITS_ARE_0,
        U64_ALL_BITS_ARE_1,
        U64_BYTES_COUNT,
        U64_HALF_BITS_COUNT,
        U64_LSBYTE_MASK,
    },
};

#[no_mangle]
pub fn arithmetic_signextend() {
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let value = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let size = u256_be_to_tuple_le(size);
    let value = u256_be_to_tuple_le(value);

    let mut res = [value.0, value.1, value.2, value.3];

    if size.0 < U64_HALF_BITS_COUNT && size.1 == 0 && size.2 == 0 && size.3 == 0 {
        let mut byte_value: u8 = 0;
        let res_idx = size.0 as usize / 8;
        let filler: u64;
        let shift = (size.0 - res_idx as u64 * U64_BYTES_COUNT) * BITS_IN_BYTE;
        byte_value = ((res[res_idx] >> shift) & U64_LSBYTE_MASK) as u8;
        if byte_value >= BYTE_SIGN_BIT_MASK as u8 {
            let shift = shift + BITS_IN_BYTE;
            res[res_idx] = (U64_ALL_BITS_ARE_1 << shift) | res[res_idx];
            filler = U64_ALL_BITS_ARE_1;
        } else {
            let shift = (7 - size.0) * BITS_IN_BYTE;
            res[res_idx] = (U64_ALL_BITS_ARE_1 >> shift) & res[res_idx];
            filler = U64_ALL_BITS_ARE_0;
        }
        for i in res_idx + 1..4 {
            res[i] = filler
        }
    }

    let r = (res[0], res[1], res[2], res[3]);

    let res = u256_tuple_le_to_be(r);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
