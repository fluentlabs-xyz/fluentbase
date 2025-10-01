use crate::CryptoRuntime;
use fluentbase_types::{CryptoAPI, B256};

pub fn crypto_keccak256(data: &[u8]) -> B256 {
    CryptoRuntime::keccak256(data)
}

pub fn crypto_sha256(data: &[u8]) -> B256 {
    // TODO(dmitry123): Replace with sha256_extend/sha256_compress
    CryptoRuntime::sha256(data)
}

pub fn crypto_poseidon(parameters: u32, endianness: u32, data: &[u8]) -> B256 {
    CryptoRuntime::poseidon(parameters, endianness, data)
}

pub fn crypto_blake3(data: &[u8]) -> B256 {
    CryptoRuntime::blake3(data)
}
