use crate::host::host_call_impl;

#[no_mangle]
pub fn host_staticcall() {
    host_call_impl(false, true)
}
