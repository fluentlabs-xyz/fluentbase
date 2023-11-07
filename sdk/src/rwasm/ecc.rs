extern "C" {
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
