use crate::host::host_call_impl;

#[no_mangle]
pub fn host_call() {
    host_call_impl(false, false)
}
