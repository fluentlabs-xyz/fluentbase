use crate::common::dup_n;

#[no_mangle]
pub fn stack_dup16() {
    dup_n(15)
}