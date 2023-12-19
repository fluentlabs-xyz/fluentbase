use crate::{
    common::{exp, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_exp() {
    let degree = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let val = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let degree = u256_be_to_tuple_le(degree);
    let val = u256_be_to_tuple_le(val);

    let r = exp(val, degree);

    let res = u256_tuple_le_to_be(r);

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, res);
}
