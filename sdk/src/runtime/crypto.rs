use crate::{CryptoPlatformSDK, SDK};

impl CryptoPlatformSDK for SDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        todo!("not implemented yet")
    }

    fn crypto_poseidon(data: &[u8], output: &mut [u8]) {
        todo!("not implemented yet")
    }

    fn crypto_poseidon2(
        fa_offset: *const u8,
        fb_offset: *const u8,
        domain_offset: *const u8,
        output_offset: *mut u8,
    ) {
        todo!("not implemented yet")
    }
}
