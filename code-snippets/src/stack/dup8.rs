use crate::common::dup_n;

#[no_mangle]
pub fn stack_dup8() {
    dup_n(7)
}
