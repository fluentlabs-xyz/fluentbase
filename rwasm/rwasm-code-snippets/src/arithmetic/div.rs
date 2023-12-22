use crate::{
    common::{div_le, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
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
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let a = u256_be_to_tuple_le(a);
    let b = u256_be_to_tuple_le(b);

    let r = div_le(a, b);

    let res = u256_tuple_le_to_be(r);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
