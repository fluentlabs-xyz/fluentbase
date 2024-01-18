use crate::common::dup_n;

#[no_mangle]
pub fn stack_dup2() {
    dup_n(1)
}
