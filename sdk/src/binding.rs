use crate::{HALT_CODE_EXIT, HALT_CODE_PANIC};

extern "C" {
    // sys
    fn _sys_halt(code: u32);
    fn _sys_read(target: *mut u8, offset: u32, length: u32) -> u32;
    fn _sys_write(offset: u32, length: u32);
    fn _sys_input(index: u32, target: u32, offset: u32, length: u32) -> i32;
    // rwasm
    fn _rwasm_compile(
        input_ptr: *const u8,
        input_len: i32,
        output_ptr: *mut u8,
        output_len: i32,
    ) -> i32;
    fn _rwasm_transact(
        code_offset: i32,
        code_len: i32,
        input_offset: i32,
        input_len: i32,
        output_offset: i32,
        output_len: i32,
    ) -> i32;
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
    fn _crypto_keccak256(data_offset: *const u8, data_len: i32, output_offset: *mut u8);
    fn _crypto_poseidon(data_offset: *const u8, data_len: i32, output_offset: *mut u8);
    fn _crypto_poseidon2(
        fa_offset: *const u8,
        fb_offset: *const u8,
        domain_offset: *const u8,
        output_offset: *mut u8,
    );
    // ecc
    fn _ecc_secp256k1_verify(
        digest: *const u8,
        digest_len: i32,
        signature: *const u8,
        signature_len: i32,
        pk_expected: *const u8,
        pk_expected_len: i32,
        rec_id: i32,
    ) -> i32;
    fn _ecc_secp256k1_recover(
        digest: *const u8,
        digest_len: i32,
        signature: *const u8,
        signature_len: i32,
        output: *mut u8,
        output_len: i32,
        rec_id: i32,
    ) -> i32;
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
pub fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
    unsafe { _crypto_keccak256(data.as_ptr(), data.len() as i32, output.as_mut_ptr()) }
}

#[inline(always)]
pub fn crypto_poseidon(data: &[u8], output: &mut [u8]) {
    unsafe { _crypto_poseidon(data.as_ptr(), data.len() as i32, output.as_mut_ptr()) }
}

#[inline(always)]
pub fn crypto_poseidon2(
    fa_offset: *const u8,
    fb_offset: *const u8,
    domain_offset: *const u8,
    output_offset: *mut u8,
) {
    unsafe { _crypto_poseidon2(fa_offset, fb_offset, domain_offset, output_offset) }
}

#[inline(always)]
pub fn ecc_secp256k1_verify(
    digest: *const u8,
    digest_len: usize,
    sig: *const u8,
    sig_len: usize,
    pk_expected: *const u8,
    pk_expected_len: usize,
    rec_id: i32,
) -> i32 {
    unsafe {
        _ecc_secp256k1_verify(
            digest,
            digest_len as i32,
            sig,
            sig_len as i32,
            pk_expected,
            pk_expected_len as i32,
            rec_id,
        )
    }
}

#[inline(always)]
pub fn ecc_secp256k1_recover(
    digest: *const u8,
    digest_len: usize,
    signature: *const u8,
    signature_len: usize,
    output: *mut u8,
    output_len: usize,
    rec_id: i32,
) -> i32 {
    unsafe {
        _ecc_secp256k1_recover(
            digest,
            digest_len as i32,
            signature,
            signature_len as i32,
            output,
            output_len as i32,
            rec_id,
        )
    }
}

#[inline(always)]
pub fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
    unsafe {
        _rwasm_compile(
            input.as_ptr(),
            input.len() as i32,
            output.as_mut_ptr(),
            output.len() as i32,
        )
    }
}

#[inline(always)]
pub fn rwasm_transact(
    code_offset: i32,
    code_len: i32,
    input_offset: i32,
    input_len: i32,
    output_offset: i32,
    output_len: i32,
) -> i32 {
    unsafe {
        _rwasm_transact(
            code_offset,
            code_len,
            input_offset,
            input_len,
            output_offset,
            output_len,
        )
    }
}
