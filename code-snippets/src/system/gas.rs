use crate::{
    common::u256_from_be_slice,
    common_sp::{stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};

#[no_mangle]
pub fn system_gas() {
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_from_be_slice(&[0]));
}
