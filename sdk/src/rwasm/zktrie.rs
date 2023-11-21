use std::vec;

use crate::{ZktriePlatformSDK, SDK};

extern "C" {
    fn _zktrie_open();
    fn _zktrie_update(key_offset: *const u8, key_len: u32, value_offset: *const u8, value_len: u32);
    fn _zktrie_delete(key_offset: *const u8, key_len: u32);
    fn _zktrie_root(root_offset: *mut u8);
}

impl ZktriePlatformSDK for SDK {
    #[inline(always)]
    fn zktrie_open() {
        unsafe { _zktrie_open() }
    }

    #[inline(always)]
    fn zktrie_update(key: &[u8], value: &[u8]) {
        unsafe {
            _zktrie_update(
                key.as_ptr(),
                key.len() as u32,
                value.as_ptr(),
                value.len() as u32,
            )
        }
    }
    #[inline(always)]
    fn zktrie_delete(key: &[u8]) {
        unsafe { _zktrie_delete(key.as_ptr(), key.len() as u32) }
    }

    #[inline(always)]
    fn zktrie_root() -> [u8; 32] {
        let mut res: [u8; 32] = [0u8; 32];
        unsafe {
            _zktrie_root(res.as_mut_ptr());
        }
        res
    }
}
