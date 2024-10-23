use crate::{b256, Address, ContextFreeNativeAPI, NativeAPI, B256, F254, U256};

const POSEIDON_DOMAIN: F254 =
    b256!("0000000000000000000000000000000000000000000000010000000000000000");

#[inline(always)]
pub fn calc_storage_key<API: NativeAPI>(address: &Address, slot32_le_ptr: *const u8) -> B256 {
    let mut slot0 = B256::ZERO;
    let mut slot1 = B256::ZERO;
    // split slot32 into two 16 byte values (slot is always 32 bytes)
    unsafe {
        core::ptr::copy(slot32_le_ptr.offset(0), slot0.as_mut_ptr(), 16);
        core::ptr::copy(slot32_le_ptr.offset(16), slot1.as_mut_ptr(), 16);
    }
    // pad address to 32 bytes value (11 bytes to avoid 254-bit overflow)
    let mut address32 = B256::ZERO;
    address32[11..31].copy_from_slice(address.as_slice());
    // compute a storage key, where formula is `p(address, p(slot_0, slot_1))`
    let storage_key = API::poseidon_hash(&slot0, &slot1, &POSEIDON_DOMAIN);
    let storage_key = API::poseidon_hash(&address32, &storage_key, &POSEIDON_DOMAIN);
    storage_key
}

#[inline(always)]
pub fn calc_create_address<API: ContextFreeNativeAPI>(deployer: &Address, nonce: u64) -> Address {
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
pub fn calc_create2_address<API: ContextFreeNativeAPI>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, keccak256};

    struct TestContext;

    impl ContextFreeNativeAPI for TestContext {
        fn keccak256(data: &[u8]) -> B256 {
            keccak256(data)
        }

        fn sha256(_data: &[u8]) -> B256 {
            todo!()
        }

        fn poseidon(_data: &[u8]) -> F254 {
            todo!()
        }

        fn poseidon_hash(_fa: &F254, _fb: &F254, _fd: &F254) -> F254 {
            todo!()
        }

        fn ec_recover(_digest: &B256, _sig: &[u8; 64], _rec_id: u8) -> [u8; 65] {
            todo!()
        }

        fn debug_log(_message: &str) {
            todo!()
        }
    }

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
                calc_create_address::<TestContext>(&address, nonce as u64),
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
                calc_create2_address::<TestContext>(&address, &salt.into(), &hash),
                address.create2(salt, hash)
            );
        }
    }
}
