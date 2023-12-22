use crate::common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT};

#[no_mangle]
pub fn stack_pop() {
    let v = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
}
