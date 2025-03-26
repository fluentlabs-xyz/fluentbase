use crate::{Address, B256};

pub fn calc_preimage_address(preimage_hash: &B256) -> Address {
    let preimage_hash: [u8; 20] = preimage_hash.0[12..].try_into().unwrap();
    Address::from(preimage_hash)
}
