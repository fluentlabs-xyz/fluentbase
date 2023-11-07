use crate::{SysPlatformSDK, SDK};

impl SysPlatformSDK for SDK {
    fn sys_read(target: *mut u8, offset: u32, len: u32) -> u32 {
        todo!("not implemented yet")
    }

    fn sys_write(offset: *const u8, len: u32) {
        todo!("not implemented yet")
    }

    fn sys_halt(exit_code: i32) {
        todo!("not implemented yet")
    }
}
