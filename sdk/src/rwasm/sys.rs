use crate::{SysPlatformSDK, SDK};

extern "C" {
    fn _sys_halt(code: i32);
    fn _sys_read(target: *mut u8, offset: u32, length: u32) -> u32;
    fn _sys_write(offset: *const u8, length: u32);
}

impl SysPlatformSDK for SDK {
    fn sys_read(target: *mut u8, offset: u32, len: u32) -> u32 {
        unsafe { _sys_read(target, offset, len) }
    }

    fn sys_write(offset: *const u8, length: u32) {
        unsafe { _sys_write(offset, length) }
    }

    fn sys_halt(exit_code: i32) {
        unsafe { _sys_halt(exit_code) }
    }
}
