use crate::host::host_create_impl_v2;

#[no_mangle]
pub fn host_create() {
    host_create_impl_v2::<false>();
}
