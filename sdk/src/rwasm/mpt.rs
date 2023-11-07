use crate::{MptPlatformSDK, SDK};

extern "C" {
    fn _mpt_open();
    fn _mpt_update(key_offset: *const u8, key_len: i32, value_offset: *const u8, value_len: i32);
    fn _mpt_get(key_offset: *const u8, key_len: i32, output_offset: *mut u8) -> i32;
    fn _mpt_get_root(output_offset: *mut u8) -> i32;
}

impl MptPlatformSDK for SDK {
    #[inline(always)]
    fn mpt_open() {
        unsafe { _mpt_open() }
    }

    #[inline(always)]
    fn mpt_update(key: &[u8], value: &[u8]) {
        unsafe {
            _mpt_update(
                key.as_ptr(),
                key.len() as i32,
                value.as_ptr(),
                value.len() as i32,
            )
        }
    }

    #[inline(always)]
    fn mpt_get(key: &[u8], output: &mut [u8]) -> i32 {
        unsafe { _mpt_get(key.as_ptr(), key.len() as i32, output.as_mut_ptr()) }
    }

    #[inline(always)]
    fn mpt_root(output: &mut [u8]) -> i32 {
        unsafe { _mpt_get_root(output.as_mut_ptr()) }
    }
}
