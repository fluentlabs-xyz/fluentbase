use crate::{runtime::LowLevelSDK, sdk::LowLevelCryptoSDK};
use fluentbase_runtime::instruction::{
    crypto_ecrecover::CryptoEcrecover,
    crypto_keccak256::CryptoKeccak256,
    crypto_poseidon::CryptoPoseidon,
    crypto_poseidon2::CryptoPoseidon2,
};

impl LowLevelCryptoSDK for LowLevelSDK {
    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        let result = CryptoKeccak256::fn_impl(data);
        output.copy_from_slice(&result);
    }

    fn crypto_poseidon(data: &[u8], output: &mut [u8]) {
        let result = CryptoPoseidon::fn_impl(data);
        output.copy_from_slice(&result);
    }

    fn crypto_poseidon2(
        fa_data: &[u8; 32],
        fb_data: &[u8; 32],
        fd_data: &[u8; 32],
        output: &mut [u8],
    ) -> bool {
        match CryptoPoseidon2::fn_impl(fa_data, fb_data, fd_data) {
            Ok(result) => {
                output.copy_from_slice(&result);
                true
            }
            Err(_) => false,
        }
    }

    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) {
        let result = CryptoEcrecover::fn_impl(digest, sig, rec_id as u32);
        output.copy_from_slice(&result);
    }
}
