use crate::host::host_log;

#[no_mangle]
pub fn host_log4() {
    host_log::<4>();
}
