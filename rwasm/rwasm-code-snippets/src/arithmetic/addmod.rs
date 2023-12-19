use crate::{
    common::{add, mod_impl, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_addmod() {
    let divisor = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let a = u256_be_to_tuple_le(a);
    let b = u256_be_to_tuple_le(b);
    let divisor = u256_be_to_tuple_le(divisor);

    let r = add(a, b);
    let r = mod_impl(r, divisor);

    let res = u256_tuple_le_to_be(r);

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, res);
}
