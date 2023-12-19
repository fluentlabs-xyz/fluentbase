use crate::common_sp::{u256_pop, SP_VAL_MEM_OFFSET_DEFAULT};

#[no_mangle]
pub fn stack_pop() {
    let v = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
}
