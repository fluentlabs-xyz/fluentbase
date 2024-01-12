use crate::{
    common::{smod, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_smod() {
    let dividend = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let divisor = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let divisor = u256_be_to_tuple_le(divisor);
    let dividend = u256_be_to_tuple_le(dividend);

    let r = smod(dividend, divisor);

    let res = u256_tuple_le_to_be(r);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
