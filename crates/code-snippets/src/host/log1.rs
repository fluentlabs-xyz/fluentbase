use crate::host::host_log;

#[no_mangle]
pub fn host_log1() {
    host_log::<1>();
}
