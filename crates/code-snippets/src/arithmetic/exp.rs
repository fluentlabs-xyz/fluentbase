use crate::{
    common::{exp, u256_be_to_u64tuple_le, u256_u64tuple_le_to_be},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_exp() {
    let val = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let degree = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let degree = u256_be_to_u64tuple_le(degree);
    let val = u256_be_to_u64tuple_le(val);

    let r = exp(val, degree);

    let res = u256_u64tuple_le_to_be(r);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
