use crate::{LowLevelAPI, LowLevelSDK};
use fluentbase_types::{b256, Address, Bytes32, B256, U256};
use revm_primitives::alloy_primitives::private::alloy_rlp::{
    Encodable, EMPTY_LIST_CODE, EMPTY_STRING_CODE,
};

const DOMAIN: [u8; 32] =
    b256!("0000000000000000000000000000000000000000000000010000000000000000").0;

#[inline(always)]
pub fn calc_storage_key(address: Address, slot32_le_ptr: *const u8) -> [u8; 32] {
    let mut slot0: [u8; 32] = [0u8; 32];
    let mut slot1: [u8; 32] = [0u8; 32];
    // split slot32 into two 16 byte values (slot is always 32 bytes)
    unsafe {
        core::ptr::copy(slot32_le_ptr.offset(0), slot0.as_mut_ptr(), 16);
        core::ptr::copy(slot32_le_ptr.offset(16), slot1.as_mut_ptr(), 16);
    }
    // pad address to 32 bytes value (11 bytes to avoid 254 overflow)
    let mut address32: [u8; 32] = [0u8; 32];
    address32[11..31].copy_from_slice(address.as_slice());
    // compute a storage key, where formula is `p(address, p(slot_0, slot_1))`
    let mut storage_key: [u8; 32] = [0u8; 32];
    LowLevelSDK::crypto_poseidon2(
        slot0.as_ptr(),
        slot1.as_ptr(),
        DOMAIN.as_ptr(),
        storage_key.as_mut_ptr(),
    );
    LowLevelSDK::crypto_poseidon2(
        address32.as_ptr(),
        storage_key.as_ptr(),
        DOMAIN.as_ptr(),
        storage_key.as_mut_ptr(),
    );
    storage_key
}

#[inline(always)]
pub fn calc_create_address(deployer: &Address, nonce: u64) -> Address {
    use alloy_rlp::{Encodable, EMPTY_LIST_CODE, EMPTY_STRING_CODE};
    const MAX_LEN: usize = 1 + (1 + 20) + 9;
    let len = 22 + nonce.length();
    debug_assert!(len <= MAX_LEN);
    let mut out = [0u8; MAX_LEN];
    out[0] = EMPTY_LIST_CODE + len as u8 - 1;
    out[1] = EMPTY_STRING_CODE + 20;
    out[2..22].copy_from_slice(deployer.as_slice());
    Encodable::encode(&nonce, &mut &mut out[22..]);
    let mut hash = B256::ZERO;
    let out = &out[..len];
    LowLevelSDK::crypto_keccak256(out.as_ptr(), out.len() as u32, hash.as_mut_ptr());
    Address::from_word(hash)
}

#[inline(always)]
pub fn calc_create2_address(deployer: &Address, salt: &U256, init_code_hash: &B256) -> Address {
    let mut bytes = [0; 85];
    bytes[0] = 0xff;
    bytes[1..21].copy_from_slice(deployer.as_slice());
    bytes[21..53].copy_from_slice(&salt.to_be_bytes::<32>());
    bytes[53..85].copy_from_slice(init_code_hash.as_slice());
    LowLevelSDK::crypto_keccak256(bytes.as_ptr(), bytes.len() as u32, bytes.as_mut_ptr());
    let bytes32: Bytes32 = bytes[0..32].try_into().unwrap();
    Address::from_word(B256::from(bytes32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExecutionContext;
    use fluentbase_types::address;
    use revm_primitives::{b256, hex, Bytecode};

    #[test]
    fn test_create_address() {
        for (address, nonce) in [
            (address!("0000000000000000000000000000000000000000"), 0),
            (
                address!("0000000000000000000000000000000000000000"),
                u32::MIN,
            ),
            (
                address!("0000000000000000000000000000000000000000"),
                u32::MAX,
            ),
            (address!("2340820934820934820934809238402983400000"), 0),
            (
                address!("2340820934820934820934809238402983400000"),
                u32::MIN,
            ),
            (
                address!("2340820934820934820934809238402983400000"),
                u32::MAX,
            ),
        ] {
            assert_eq!(
                calc_create_address(&address, nonce as u64),
                address.create(nonce as u64)
            );
        }
    }

    #[test]
    fn test_create2_address() {
        let address = Address::ZERO;
        for (salt, hash) in [(
            b256!("0000000000000000000000000000000000000000000000000000000000000001"),
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
        )] {
            assert_eq!(
                calc_create2_address(&address, &salt.into(), &hash),
                address.create2(salt, hash)
            );
        }
    }
}
