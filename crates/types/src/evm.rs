pub use alloy_primitives::{address, b256, bloom, bytes, fixed_bytes, Address, Bytes, B256, U256};

pub const KECCAK256_EMPTY: B256 =
    b256!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
pub const POSEIDON_EMPTY: B256 =
    b256!("2098f5fb9e239eab3ceac3f27b81e481dc3124d55ffed523a839ee8446b64864");

#[derive(Clone)]
pub struct Account {
    pub balance: U256,
    pub nonce: u64,
    pub code_hash: B256,
    pub code: Option<Bytes>,
}

impl Account {
    pub fn empty_code_hash() -> B256 {
        KECCAK256_EMPTY
    }
}

impl Default for Account {
    fn default() -> Self {
        Self {
            balance: U256::ZERO,
            code_hash: Self::empty_code_hash(),
            code: Some(Bytes::new()),
            nonce: 0,
        }
    }
}
