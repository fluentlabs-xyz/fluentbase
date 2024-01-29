use crate::{
    common::{div_le, u256_be_to_u64tuple_le, u256_u64tuple_le_to_be},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_div() {
    let a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let a = u256_be_to_u64tuple_le(a);
    let b = u256_be_to_u64tuple_le(b);

    let r = div_le(a, b);

    let res = u256_u64tuple_le_to_be(r);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
