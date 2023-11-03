use crate::{HALT_CODE_EXIT, HALT_CODE_PANIC};

extern "C" {
    // sys
    fn _sys_halt(code: u32);
    fn _sys_read(target: *mut u8, offset: u32, length: u32);
    fn _sys_write(offset: u32, length: u32);
    // zktrie
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
    // mpt
    pub fn _mpt_open();
    pub fn _mpt_update(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32);
    pub fn _mpt_get(key_offset: i32, key_len: i32, output_offset: i32) -> i32;
    pub fn _mpt_get_root(output_offset: i32) -> i32;
    // crypto
    pub fn _crypto_keccak256(data_offset: i32, data_len: i32, output_offset: i32) -> i32;
    pub fn _crypto_poseidon(data_offset: i32, data_len: i32, output_offset: i32) -> i32;
    pub fn _crypto_poseidon2(
        fa_offset: i32,
        fb_offset: i32,
        fdomain_offset: i32,
        output_offset: i32,
    ) -> i32;
    pub fn _crypto_secp256k1_verify(
        digest: i32,
        digest_len: i32,
        sig: i32,
        sig_len: i32,
        recid: i32,
        pk_expected: i32,
        pk_expected_len: i32,
    ) -> i32;
}

#[inline(always)]
pub fn sys_read(target: *mut u8, offset: u32, len: u32) {
    unsafe { _sys_read(target, offset, len) }
}

#[inline(always)]
pub fn sys_write(offset: u32, len: u32) {
    unsafe { _sys_write(offset, len) }
}

#[inline(always)]
pub fn sys_read_slice(target: &mut [u8], offset: u32) {
    unsafe { _sys_read(target.as_mut_ptr(), offset, target.len() as u32) }
}

#[inline(always)]
pub fn sys_exit() {
    unsafe { _sys_halt(HALT_CODE_EXIT) }
}

#[inline(always)]
pub fn sys_panic() {
    unsafe { _sys_halt(HALT_CODE_PANIC) }
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

#[inline(always)]
pub fn mpt_open() {
    unsafe { _mpt_open() }
}

#[inline(always)]
pub fn mpt_update(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) {
    unsafe { _mpt_update(key_offset, key_len, value_offset, value_len) }
}

#[inline(always)]
pub fn mpt_get(key_offset: i32, key_len: i32, output_offset: i32) -> i32 {
    unsafe { _mpt_get(key_offset, key_len, output_offset) }
}

#[inline(always)]
pub fn mpt_get_root(output_offset: i32) -> i32 {
    unsafe { _mpt_get_root(output_offset) }
}

#[inline(always)]
pub fn crypto_keccak256(data_offset: i32, data_len: i32, output_offset: i32) -> i32 {
    unsafe { _crypto_keccak256(data_offset, data_len, output_offset) }
}

#[inline(always)]
pub fn crypto_poseidon(data: &[u8], output: &mut [u8]) -> i32 {
    unsafe {
        _crypto_poseidon(
            data.as_ptr() as i32,
            data.len() as i32,
            output.as_mut_ptr() as i32,
        )
    }
}

#[inline(always)]
pub fn crypto_poseidon2(
    fa_offset: i32,
    fb_offset: i32,
    fdomain_offset: i32,
    output_offset: i32,
) -> i32 {
    unsafe { _crypto_poseidon2(fa_offset, fb_offset, fdomain_offset, output_offset) }
}

#[inline(always)]
pub fn crypto_secp256k1_verify(
    digest: i32,
    digest_len: i32,
    sig: i32,
    sig_len: i32,
    recid: i32,
    pk_expected: i32,
    pk_expected_len: i32,
) -> i32 {
    unsafe {
        _crypto_secp256k1_verify(
            digest,
            digest_len,
            sig,
            sig_len,
            recid,
            pk_expected,
            pk_expected_len,
        )
    }
}
