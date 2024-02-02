use crate::host::host_create_impl;

#[no_mangle]
pub fn host_create() {
    host_create_impl(false);
}
