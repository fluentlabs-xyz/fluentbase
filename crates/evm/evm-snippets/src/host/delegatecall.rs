use crate::host::host_call_impl_v2;

#[no_mangle]
pub fn host_delegatecall() {
    host_call_impl_v2::<true, false>()
}
