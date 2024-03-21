use crate::host::host_create_impl_v2;

#[no_mangle]
pub fn host_create2() {
    host_create_impl_v2::<true>();
}
