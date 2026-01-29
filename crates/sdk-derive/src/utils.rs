pub fn calculate_keccak256_bytes<const N: usize>(signature: &str) -> [u8; N] {
    use crypto_hashes::{digest::Digest, sha3::Keccak256};
    let mut hash = Keccak256::new();
    hash.update(signature);
    let mut dst = [0u8; N];
    dst.copy_from_slice(&*hash.finalize()[0..N].as_ref());
    dst
}

pub fn calculate_keccak256_id(signature: &str) -> u32 {
    u32::from_be_bytes(calculate_keccak256_bytes::<4>(signature.trim_matches('"')))
}

pub fn calculate_keccak256(signature: &str) -> [u8; 32] {
    let mut val = [0u8; 32];
    val.copy_from_slice(calculate_keccak256_bytes::<32>(signature.trim_matches('"')).as_slice());
    val
}

pub fn calculate_keccak256_raw<const N: usize>(data: &[u8]) -> [u8; N] {
    use crypto_hashes::{digest::Digest, sha3::Keccak256};
    let mut hash = Keccak256::new();
    hash.update(data);
    let mut dst = [0u8; N];
    dst.copy_from_slice(&*hash.finalize()[0..N].as_ref());
    dst
}
