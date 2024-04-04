use crate::common_sp::{stack_push_u256, u256_zero, SP_BASE_MEM_OFFSET_DEFAULT};

#[no_mangle]
pub fn control_pc() {
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, u256_zero());
}
