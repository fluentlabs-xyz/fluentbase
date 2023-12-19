use crate::{
    common::{div_le, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_div(// b0: u64,
    // b1: u64,
    // b2: u64,
    // b3: u64,
    // a0: u64,
    // a1: u64,
    // a2: u64,
    // a3: u64,
) {
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let a = u256_be_to_tuple_le(a);
    let b = u256_be_to_tuple_le(b);

    let r = div_le(a, b);

    let res = u256_tuple_le_to_be(r);

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, res);
}
