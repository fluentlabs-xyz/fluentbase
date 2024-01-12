use crate::{
    rwasm::{
        bindings::{_crypto_ecrecover, _crypto_keccak256, _crypto_poseidon, _crypto_poseidon2},
        LowLevelSDK,
    },
    sdk::LowLevelCryptoSDK,
    types::Bytes32,
};

impl LowLevelCryptoSDK for LowLevelSDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        unsafe { _crypto_keccak256(data.as_ptr(), data.len() as i32, output.as_mut_ptr()) }
    }

    fn crypto_poseidon(fr32_data: &[u8], output: &mut [u8]) {
        unsafe {
            _crypto_poseidon(
                fr32_data.as_ptr(),
                fr32_data.len() as i32,
                output.as_mut_ptr(),
            )
        }
    }

    fn crypto_poseidon2(
        fa32_data: &Bytes32,
        fb32_data: &Bytes32,
        fd32_data: &Bytes32,
        output32: &mut [u8],
    ) -> bool {
        unsafe {
            _crypto_poseidon2(
                fa32_data.as_ptr(),
                fb32_data.as_ptr(),
                fd32_data.as_ptr(),
                output32.as_mut_ptr(),
            )
        }
    }

    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) {
        unsafe {
            _crypto_ecrecover(
                digest.as_ptr(),
                sig.as_ptr(),
                output.as_mut_ptr(),
                rec_id as u32,
            )
        }
    }
}
