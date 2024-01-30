use crate::{
    common::{mod_impl, mul, u256_be_to_u64tuple_le, u256_u64tuple_le_to_be},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_mulmod() {
    let mul1 = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mul2 = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let divisor = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let divisor = u256_be_to_u64tuple_le(divisor);
    let mul1 = u256_be_to_u64tuple_le(mul1);
    let mul2 = u256_be_to_u64tuple_le(mul2);

    let dividend = mul(mul1, mul2);
    let r = mod_impl(dividend, divisor);

    let res = u256_u64tuple_le_to_be(r);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
