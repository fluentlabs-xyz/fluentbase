use crate::host::host_create_impl;

#[no_mangle]
pub fn host_create2() {
    host_create_impl(true);
}
