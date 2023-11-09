use crate::{CryptoPlatformSDK, SDK};

extern "C" {
    fn _crypto_keccak256(data_offset: *const u8, data_len: i32, output_offset: *mut u8);
    fn _crypto_poseidon(data_offset: *const u8, data_len: i32, output_offset: *mut u8);
    fn _crypto_poseidon2(
        fa_offset: *const u8,
        fb_offset: *const u8,
        domain_offset: *const u8,
        output_offset: *mut u8,
    );
}

impl CryptoPlatformSDK for SDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        unsafe { _crypto_keccak256(data.as_ptr(), data.len() as i32, output.as_mut_ptr()) }
    }

    fn crypto_poseidon(data: &[u8], output: &mut [u8]) {
        unsafe { _crypto_poseidon(data.as_ptr(), data.len() as i32, output.as_mut_ptr()) }
    }

    fn crypto_poseidon2(
        fa_data: &[u8; 32],
        fb_data: &[u8; 32],
        domain_data: &[u8; 32],
        output: &mut [u8],
    ) {
        unsafe {
            _crypto_poseidon2(
                fa_data.as_ptr(),
                fb_data.as_ptr(),
                domain_data.as_ptr(),
                output.as_mut_ptr(),
            )
        }
    }
}
