use crate::host::host_call_impl;

#[no_mangle]
pub fn host_delegatecall() {
    host_call_impl(true, false)
}
