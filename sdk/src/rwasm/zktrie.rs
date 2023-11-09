use std::vec;

use crate::{ZktriePlatformSDK, SDK};

extern "C" {
    fn _zktrie_open();
    fn _zktrie_update_nonce(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32);
    fn _zktrie_update_balance(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32);
    fn _zktrie_update_storage_root(
        key_offset: i32,
        key_len: i32,
        value_offset: i32,
        value_len: i32,
    );
    fn _zktrie_update_code_hash(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32);
    fn _zktrie_update_code_size(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32);
    fn _zktrie_get_nonce(key_offset: i32, key_len: i32, output_offset: i32);
    fn _zktrie_get_balance(key_offset: i32, key_len: i32, output_offset: i32);
    fn _zktrie_get_storage_root(key_offset: i32, key_len: i32, output_offset: i32);
    fn _zktrie_get_code_hash(key_offset: i32, key_len: i32, output_offset: i32);
    fn _zktrie_get_code_size(key_offset: i32, key_len: i32, output_offset: i32);
    fn _zktrie_update_store(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32);
    fn _zktrie_get_store(key_offset: i32, key_len: i32, output_offset: i32);
}

impl ZktriePlatformSDK for SDK {
    #[inline(always)]
    fn zktrie_open() {
        unsafe { _zktrie_open() }
    }

    #[inline(always)]
    fn zktrie_update_nonce(key: &[u8], value: &[u8; 32]) {
        unsafe {
            _zktrie_update_nonce(
                key.as_ptr() as i32,
                key.len() as i32,
                value.as_ptr() as i32,
                value.len() as i32,
            )
        }
    }

    #[inline(always)]
    fn zktrie_update_balance(key: &[u8], value: &[u8; 32]) {
        unsafe {
            _zktrie_update_balance(
                key.as_ptr() as i32,
                key.len() as i32,
                value.as_ptr() as i32,
                value.len() as i32,
            )
        }
    }

    #[inline(always)]
    fn zktrie_update_storage_root(key: &[u8], value: &[u8; 32]) {
        unsafe {
            _zktrie_update_storage_root(
                key.as_ptr() as i32,
                key.len() as i32,
                value.as_ptr() as i32,
                value.len() as i32,
            )
        }
    }

    #[inline(always)]
    fn zktrie_update_code_hash(key: &[u8], value: &[u8; 32]) {
        unsafe {
            _zktrie_update_code_hash(
                key.as_ptr() as i32,
                key.len() as i32,
                value.as_ptr() as i32,
                value.len() as i32,
            )
        }
    }

    #[inline(always)]
    fn zktrie_update_code_size(key: &[u8], value: &[u8; 32]) {
        unsafe {
            _zktrie_update_code_size(
                key.as_ptr() as i32,
                key.len() as i32,
                value.as_ptr() as i32,
                value.len() as i32,
            )
        }
    }

    #[inline(always)]
    fn zktrie_get_nonce(key: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe {
            _zktrie_get_nonce(
                key.as_ptr() as i32,
                key.len() as i32,
                out.as_mut_ptr() as i32,
            )
        }
        out
    }

    #[inline(always)]
    fn zktrie_get_balance(key: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe {
            _zktrie_get_balance(
                key.as_ptr() as i32,
                key.len() as i32,
                out.as_mut_ptr() as i32,
            )
        }
        out
    }

    #[inline(always)]
    fn zktrie_get_storage_root(key: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe {
            _zktrie_get_storage_root(
                key.as_ptr() as i32,
                key.len() as i32,
                out.as_mut_ptr() as i32,
            )
        }
        out
    }

    #[inline(always)]
    fn zktrie_get_code_hash(key: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe {
            _zktrie_get_code_hash(
                key.as_ptr() as i32,
                key.len() as i32,
                out.as_mut_ptr() as i32,
            )
        }
        out
    }

    #[inline(always)]
    fn zktrie_get_code_size(key: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe {
            _zktrie_get_code_size(
                key.as_ptr() as i32,
                key.len() as i32,
                out.as_mut_ptr() as i32,
            )
        }
        out
    }

    #[inline(always)]
    fn zktrie_update_store(key: &[u8], value: &[u8; 32]) {
        unsafe {
            _zktrie_update_store(
                key.as_ptr() as i32,
                key.len() as i32,
                value.as_ptr() as i32,
                value.len() as i32,
            )
        }
    }

    #[inline(always)]
    fn zktrie_get_store(key: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe {
            _zktrie_get_store(
                key.as_ptr() as i32,
                key.len() as i32,
                out.as_mut_ptr() as i32,
            )
        }
        out
    }
}
