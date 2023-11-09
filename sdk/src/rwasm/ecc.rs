use crate::{EccPlatformSDK, SDK};

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
        sig: *const u8,
        sig_len: i32,
        output: *mut u8,
        output_len: i32,
        rec_id: i32,
    ) -> i32;
}

impl EccPlatformSDK for SDK {
    fn ecc_secp256k1_verify(digest: &[u8], sig: &[u8], pk_expected: &[u8], rec_id: u8) -> bool {
        unsafe {
            _ecc_secp256k1_verify(
                digest.as_ptr(),
                digest.len() as i32,
                sig.as_ptr(),
                sig.len() as i32,
                pk_expected.as_ptr(),
                pk_expected.len() as i32,
                rec_id as i32,
            ) != 0
        }
    }

    fn ecc_secp256k1_recover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) -> bool {
        unsafe {
            _ecc_secp256k1_recover(
                digest.as_ptr(),
                digest.len() as i32,
                sig.as_ptr(),
                sig.len() as i32,
                output.as_mut_ptr(),
                output.len() as i32,
                rec_id as i32,
            ) != 0
        }
    }
}
