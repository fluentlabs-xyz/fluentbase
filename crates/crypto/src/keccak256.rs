use fluentbase_types::B256;

pub fn crypto_keccak256(data: &[u8]) -> B256 {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    B256::from(out)
}
