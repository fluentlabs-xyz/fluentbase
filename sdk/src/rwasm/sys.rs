extern "C" {
    fn _sys_halt(code: i32);
    fn _sys_read(target: *mut u8, offset: u32, length: u32) -> u32;
    fn _sys_write(offset: u32, length: u32);
    fn _sys_input(index: u32, target: u32, offset: u32, length: u32) -> i32;
}

#[inline(always)]
pub fn sys_read(target: *mut u8, offset: u32, len: u32) -> u32 {
    unsafe { _sys_read(target, offset, len) }
}

#[inline(always)]
pub fn sys_write(offset: u32, len: u32) {
    unsafe { _sys_write(offset, len) }
}

#[inline(always)]
pub fn sys_input(index: u32, target: u32, offset: u32, length: u32) -> i32 {
    unsafe { _sys_input(index, target, offset, length) }
}

#[inline(always)]
pub fn sys_halt(exit_code: i32) {
    unsafe { _sys_halt(exit_code) }
}
