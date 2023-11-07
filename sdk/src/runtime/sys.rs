use crate::{SysPlatformSDK, SDK};

impl SysPlatformSDK for SDK {
    fn sys_read(target: &mut [u8], offset: u32) -> u32 {
        unreachable!("I think this function is not possible for runtime")
    }

    fn sys_write(value: &[u8]) {
        unreachable!("I think this function is not possible for runtime")
    }

    fn sys_halt(exit_code: i32) {
        unreachable!("I think this function is not possible for runtime")
    }
}
