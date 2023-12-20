use crate::{
    common::{mul, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_mul() {
    let mul1 = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let mul2 = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let mul1 = u256_be_to_tuple_le(mul1);
    let mul2 = u256_be_to_tuple_le(mul2);

    let r = mul(mul1, mul2);

    let res = u256_tuple_le_to_be(r);

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, res);
}
