extern "C" {
    fn _mpt_open();
    fn _mpt_update(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32);
    fn _mpt_get(key_offset: i32, key_len: i32, output_offset: i32) -> i32;
    fn _mpt_get_root(output_offset: i32) -> i32;
}

#[inline(always)]
pub fn mpt_open() {
    unsafe { _mpt_open() }
}

#[inline(always)]
pub fn mpt_update(key: &[u8], value: &[u8]) {
    unsafe {
        _mpt_update(
            key.as_ptr() as i32,
            key.len() as i32,
            value.as_ptr() as i32,
            value.len() as i32,
        )
    }
}

#[inline(always)]
pub fn mpt_get(key_offset: i32, key_len: i32, output_offset: i32) -> i32 {
    unsafe { _mpt_get(key_offset, key_len, output_offset) }
}

#[inline(always)]
pub fn mpt_get_root(output_offset: i32) -> i32 {
    unsafe { _mpt_get_root(output_offset) }
}
