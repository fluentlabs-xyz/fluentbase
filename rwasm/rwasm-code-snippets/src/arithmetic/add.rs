use crate::{
    common::{add, u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn arithmetic_add() {
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let a = u256_be_to_tuple_le(a);
    let b = u256_be_to_tuple_le(b);

    let r = add(a, b);

    let res = u256_tuple_le_to_be(r);

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, res);
}
