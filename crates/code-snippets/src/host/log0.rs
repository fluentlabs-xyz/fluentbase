use crate::host::host_log;

#[no_mangle]
pub fn host_log0() {
    host_log::<0>();
}
