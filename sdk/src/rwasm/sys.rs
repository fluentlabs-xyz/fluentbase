use crate::{SysPlatformSDK, SDK};

extern "C" {
    fn _sys_halt(code: i32);
    fn _sys_read(target: *mut u8, offset: u32, length: u32) -> u32;
    fn _sys_write(offset: *const u8, length: u32);
}

impl SysPlatformSDK for SDK {
    fn sys_read(target: &mut [u8], offset: u32) -> u32 {
        unsafe { _sys_read(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    fn sys_write(value: &[u8]) {
        unsafe { _sys_write(value.as_ptr(), value.len() as u32) }
    }

    fn sys_halt(exit_code: i32) {
        unsafe { _sys_halt(exit_code) }
    }
}
