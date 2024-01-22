use crate::{
    common::{mul, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_mul() {
    let mul1 = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mul2 = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let mul1 = u256_be_to_tuple_le(mul1);
    let mul2 = u256_be_to_tuple_le(mul2);

    let r = mul(mul1, mul2);

    let res = u256_tuple_le_to_be(r);

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
