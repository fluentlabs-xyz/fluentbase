use crate::{HALT_CODE_EXIT, HALT_CODE_PANIC};

extern {
    fn _sys_halt(code: u32);
    fn _sys_write(source: *const u8, len: u32);
    fn _sys_read(target: *mut u8, offset: u32, len: u32);
}

#[inline(always)]
pub fn sys_write(source: *const u8, len: u32) {
    unsafe {
        _sys_write(source, len)
    }
}

#[inline(always)]
pub fn sys_write_slice(source: &[u8]) {
    unsafe {
        _sys_write(source.as_ptr(), source.len() as u32)
    }
}

#[inline(always)]
pub fn sys_read(target: *mut u8, offset: u32, len: u32) {
    unsafe {
        _sys_read(target, offset, len)
    }
}

#[inline(always)]
pub fn sys_read_slice(target: &mut [u8], offset: u32) {
    unsafe {
        _sys_read(target.as_mut_ptr(), offset, target.len() as u32)
    }
}

#[inline(always)]
pub fn sys_exit() {
    unsafe {
        _sys_halt(HALT_CODE_EXIT)
    }
}

#[inline(always)]
pub fn sys_panic() {
    unsafe {
        _sys_halt(HALT_CODE_PANIC)
    }
}