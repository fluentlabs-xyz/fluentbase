use crate::host::host_log;

#[no_mangle]
pub fn host_log2() {
    host_log::<2>();
}
