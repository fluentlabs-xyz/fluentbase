use crate::{SysPlatformSDK, SDK};

impl SysPlatformSDK for SDK {
    fn sys_read(_target: &mut [u8], _offset: u32) -> u32 {
        unreachable!("I think this function is not possible for runtime")
    }

    fn sys_write(_value: &[u8]) {
        unreachable!("I think this function is not possible for runtime")
    }

    fn sys_halt(_exit_code: i32) {
        unreachable!("I think this function is not possible for runtime")
    }
}
