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
    todo!("not implemented yet")
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
    todo!("not implemented yet")
}
