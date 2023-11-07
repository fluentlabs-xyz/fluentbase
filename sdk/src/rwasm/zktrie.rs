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

#[inline(always)]
pub fn zktrie_open() {
    unsafe { _zktrie_open() }
}

#[inline(always)]
pub fn zktrie_update_nonce(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) {
    unsafe { _zktrie_update_nonce(key_offset, key_len, value_offset, value_len) }
}

#[inline(always)]
pub fn zktrie_update_balance(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) {
    unsafe { _zktrie_update_balance(key_offset, key_len, value_offset, value_len) }
}

#[inline(always)]
pub fn zktrie_update_storage_root(
    key_offset: i32,
    key_len: i32,
    value_offset: i32,
    value_len: i32,
) {
    unsafe { _zktrie_update_storage_root(key_offset, key_len, value_offset, value_len) }
}

#[inline(always)]
pub fn zktrie_update_code_hash(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) {
    unsafe { _zktrie_update_code_hash(key_offset, key_len, value_offset, value_len) }
}

#[inline(always)]
pub fn zktrie_update_code_size(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) {
    unsafe { _zktrie_update_code_size(key_offset, key_len, value_offset, value_len) }
}

#[inline(always)]
pub fn zktrie_get_nonce(key_offset: i32, key_len: i32, output_offset: i32) {
    unsafe { _zktrie_get_nonce(key_offset, key_len, output_offset) }
}

#[inline(always)]
pub fn zktrie_get_balance(key_offset: i32, key_len: i32, output_offset: i32) {
    unsafe { _zktrie_get_balance(key_offset, key_len, output_offset) }
}

#[inline(always)]
pub fn zktrie_get_storage_root(key_offset: i32, key_len: i32, output_offset: i32) {
    unsafe { _zktrie_get_storage_root(key_offset, key_len, output_offset) }
}

#[inline(always)]
pub fn zktrie_get_code_hash(key_offset: i32, key_len: i32, output_offset: i32) {
    unsafe { _zktrie_get_code_hash(key_offset, key_len, output_offset) }
}

#[inline(always)]
pub fn zktrie_get_code_size(key_offset: i32, key_len: i32, output_offset: i32) {
    unsafe { _zktrie_get_code_size(key_offset, key_len, output_offset) }
}

#[inline(always)]
pub fn zktrie_update_store(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) {
    unsafe { _zktrie_update_store(key_offset, key_len, value_offset, value_len) }
}

#[inline(always)]
pub fn zktrie_get_store(key_offset: i32, key_len: i32, output_offset: i32) {
    unsafe { _zktrie_get_store(key_offset, key_len, output_offset) }
}
