use crate::{
    common::{smod, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_smod() {
    let divisor = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let dividend = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let divisor = u256_be_to_tuple_le(divisor);
    let dividend = u256_be_to_tuple_le(dividend);

    let r = smod(dividend, divisor);

    let res = u256_tuple_le_to_be(r);

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, res);
}
