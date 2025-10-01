use crate::{Address, CryptoAPI, B256, U256};

#[inline(always)]
pub fn calc_create_address<API: CryptoAPI>(deployer: &Address, nonce: u64) -> Address {
    use alloy_rlp::{Encodable, EMPTY_LIST_CODE, EMPTY_STRING_CODE};
    const MAX_LEN: usize = 1 + (1 + 20) + 9;
    let len = 22 + nonce.length();
    debug_assert!(len <= MAX_LEN);
    let mut out = [0u8; MAX_LEN];
    out[0] = EMPTY_LIST_CODE + len as u8 - 1;
    out[1] = EMPTY_STRING_CODE + 20;
    out[2..22].copy_from_slice(deployer.as_slice());
    Encodable::encode(&nonce, &mut &mut out[22..]);
    let out = &out[..len];
    Address::from_word(API::keccak256(&out))
}

#[inline(always)]
pub fn calc_create2_address<API: CryptoAPI>(
    deployer: &Address,
    salt: &U256,
    init_code_hash: &B256,
) -> Address {
    let mut bytes = [0; 85];
    bytes[0] = 0xff;
    bytes[1..21].copy_from_slice(deployer.as_slice());
    bytes[21..53].copy_from_slice(&salt.to_be_bytes::<32>());
    bytes[53..85].copy_from_slice(init_code_hash.as_slice());
    let hash = API::keccak256(&bytes);
    Address::from_word(hash)
}

#[inline(always)]
pub fn calc_create4_address(owner: &Address, salt: &U256, hash_func: fn(&[u8]) -> B256) -> Address {
    let mut bytes = [0; 53];
    bytes[0] = 0x44;
    bytes[1..21].copy_from_slice(owner.as_slice());
    bytes[21..53].copy_from_slice(&salt.to_be_bytes::<32>());
    let hash = hash_func(&bytes);
    Address::from_word(hash)
}
