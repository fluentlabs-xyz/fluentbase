use fluentbase_types::B256;

pub fn crypto_keccak256<T: AsRef<[u8]>>(data: T) -> B256 {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    hasher.update(data.as_ref());
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    B256::from(out)
}
