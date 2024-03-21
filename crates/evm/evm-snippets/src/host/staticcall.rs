use crate::host::host_call_impl_v2;

#[no_mangle]
pub fn host_staticcall() {
    host_call_impl_v2::<false, true>()
}
