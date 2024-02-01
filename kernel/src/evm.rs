mod balance;
mod create;
mod selfbalance;

use byteorder::{BigEndian, ByteOrder};
use core::ptr;
use fluentbase_sdk::{
    evm::{Address, B256, U256},
    Bytes32,
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{KECCAK_EMPTY, POSEIDON_EMPTY};

pub(crate) const ZKTRIE_CODESIZE_NONCE_FIELD: u32 = 1;
pub(crate) const ZKTRIE_BALANCE_FIELD: u32 = 2;
pub(crate) const ZKTRIE_ROOT_FIELD: u32 = 3;
pub(crate) const ZKTRIE_KECCAK_CODE_HASH_FIELD: u32 = 4;
pub(crate) const ZKTRIE_CODE_HASH_FIELD: u32 = 5;

/// EIP-170: Contract code size limit
///
/// By default this limit is 0x6000 (~24kb)
pub(crate) const MAX_CODE_SIZE: u32 = 0x6000;

pub(crate) struct Account {
    code_size: u64,
    nonce: u64,
    balance: U256,
    root: B256,
    keccak_code_hash: B256,
    code_hash: B256,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            code_size: 0,
            nonce: 0,
            balance: U256::ZERO,
            root: B256::ZERO,
            keccak_code_hash: KECCAK_EMPTY,
            code_hash: POSEIDON_EMPTY,
        }
    }
}

impl Account {
    #[inline(always)]
    fn read_account(address: &Address) -> Self {
        let mut result = Self::default();
        let mut bytes32 = [0u8; 32];
        unsafe {
            ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
        }
        let mut code_size_nonce = [0u8; 32];
        LowLevelSDK::zktrie_field(
            bytes32.as_ptr(),
            ZKTRIE_CODESIZE_NONCE_FIELD,
            code_size_nonce.as_mut_ptr(),
        );
        result.code_size = BigEndian::read_u64(&code_size_nonce[16..]);
        result.nonce = BigEndian::read_u64(&code_size_nonce[24..]);
        LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, unsafe {
            result.balance.as_le_slice_mut().as_mut_ptr()
        });
        LowLevelSDK::zktrie_field(
            bytes32.as_ptr(),
            ZKTRIE_ROOT_FIELD,
            result.root.as_mut_ptr(),
        );
        LowLevelSDK::zktrie_field(
            bytes32.as_ptr(),
            ZKTRIE_KECCAK_CODE_HASH_FIELD,
            result.keccak_code_hash.as_mut_ptr(),
        );
        LowLevelSDK::zktrie_field(
            bytes32.as_ptr(),
            ZKTRIE_CODE_HASH_FIELD,
            result.code_hash.as_mut_ptr(),
        );
        result
    }

    #[inline(always)]
    fn commit(&self, address: &Address) {
        let mut values: [Bytes32; 5] = [[0u8; 32]; 5];
        BigEndian::write_u64(&mut values[0][16..], self.code_size);
        BigEndian::write_u64(&mut values[0][24..], self.nonce);
        values[1].copy_from_slice(&self.balance.to_be_bytes::<32>());
        values[2].copy_from_slice(self.root.as_slice());
        values[3].copy_from_slice(self.keccak_code_hash.as_slice());
        values[4].copy_from_slice(self.code_hash.as_slice());
        let mut key32 = [0u8; 32];
        unsafe {
            ptr::copy(address.as_ptr(), key32.as_mut_ptr(), 20);
        }
        LowLevelSDK::zktrie_update(&key32, 8, &values);
    }

    #[inline(always)]
    fn is_not_empty(&self) -> bool {
        self.nonce != 0 || self.keccak_code_hash != KECCAK_EMPTY || self.code_hash != POSEIDON_EMPTY
    }

    #[inline(always)]
    fn transfer_value(&mut self, to: &mut Self, value: &U256) -> bool {
        self.balance.checked_sub(*value).is_some() && to.balance.checked_add(*value).is_some()
    }
}

#[inline(always)]
fn read_input_address(offset: usize) -> Address {
    let mut address = [0u8; Address::len_bytes()];
    LowLevelSDK::sys_read(&mut address, offset as u32);
    Address::from(address)
}

#[inline(always)]
fn read_balance(address: Address, value: &mut U256) {
    let mut bytes32 = [0u8; 32];
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, unsafe {
        value.as_le_slice_mut().as_mut_ptr()
    });
}

#[inline(always)]
fn write_balance(address: Address, value: &mut U256) {
    let mut bytes32 = [0u8; 32];
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, unsafe {
        value.as_le_slice_mut().as_mut_ptr()
    });
}

#[inline(always)]
fn calc_create_address(deployer: &Address, nonce: u64) -> Address {
    use alloy_rlp::{Encodable, EMPTY_LIST_CODE, EMPTY_STRING_CODE};
    const MAX_LEN: usize = 1 + (1 + 20) + 9;
    let len = 22 + nonce.length();
    debug_assert!(len <= MAX_LEN);
    let mut out = [0u8; MAX_LEN + 1];
    out[0] = EMPTY_LIST_CODE + len as u8 - 1;
    out[1] = EMPTY_STRING_CODE + 20;
    out[2..22].copy_from_slice(deployer.as_slice());
    nonce.encode(&mut &mut out[22..]);
    LowLevelSDK::crypto_keccak256(out.as_ptr(), out.len() as u32, out.as_mut_ptr());
    Address::from_word(B256::from(out))
}

#[inline(always)]
fn calc_create2_address(deployer: &Address, salt: &B256, init_code_hash: &B256) -> Address {
    let mut bytes = [0; 85];
    bytes[0] = 0xff;
    bytes[1..21].copy_from_slice(deployer.as_slice());
    bytes[21..53].copy_from_slice(salt.as_slice());
    bytes[53..85].copy_from_slice(init_code_hash.as_slice());
    LowLevelSDK::crypto_keccak256(bytes.as_ptr(), bytes.len() as u32, bytes.as_mut_ptr());
    let bytes32: [u8; 32] = bytes[0..32].try_into().unwrap();
    Address::from_word(B256::from(bytes32))
}

#[cfg(test)]
mod tests {
    use crate::evm::calc_create_address;
    use alloc::vec;
    use fluentbase_sdk::evm::Address;
    use fluentbase_types::{address, B256};
    use keccak_hash::keccak;

    #[test]
    fn create_correctness() {
        fn create_slow(address: &Address, nonce: u64) -> Address {
            use alloy_rlp::Encodable;
            let mut out = vec![];
            alloy_rlp::Header {
                list: true,
                payload_length: address.length() + nonce.length(),
            }
            .encode(&mut out);
            address.encode(&mut out);
            nonce.encode(&mut out);
            Address::from_word(keccak(out).0.into())
        }
        let tests = vec![(address!("0000000000000000000000000000000000000000"), 100)];
        for (address, nonce) in tests {
            assert_eq!(
                calc_create_address(&address, nonce),
                create_slow(&address, nonce)
            )
        }
    }

    #[test]
    fn create2() {
        let tests = [
            (
                "0000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "00",
                "4D1A2e2bB4F88F0250f26Ffff098B0b30B26BF38",
            ),
            (
                "deadbeef00000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "00",
                "B928f69Bb1D91Cd65274e3c79d8986362984fDA3",
            ),
            (
                "deadbeef00000000000000000000000000000000",
                "000000000000000000000000feed000000000000000000000000000000000000",
                "00",
                "D04116cDd17beBE565EB2422F2497E06cC1C9833",
            ),
            (
                "0000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "deadbeef",
                "70f2b2914A2a4b783FaEFb75f459A580616Fcb5e",
            ),
            (
                "00000000000000000000000000000000deadbeef",
                "00000000000000000000000000000000000000000000000000000000cafebabe",
                "deadbeef",
                "60f3f640a8508fC6a86d45DF051962668E1e8AC7",
            ),
            (
                "00000000000000000000000000000000deadbeef",
                "00000000000000000000000000000000000000000000000000000000cafebabe",
                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
                "1d8bfDC5D46DC4f61D6b6115972536eBE6A8854C",
            ),
            (
                "0000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "",
                "E33C0C7F7df4809055C3ebA6c09CFe4BaF1BD9e0",
            ),
        ];
        for (from, salt, init_code, expected) in tests {
            let from = from.parse::<Address>().unwrap();

            let salt = hex::decode(salt).unwrap();
            let salt: [u8; 32] = salt.try_into().unwrap();

            let init_code = hex::decode(init_code).unwrap();
            let init_code_hash: B256 = keccak(&init_code).0.into();

            let expected = expected.parse::<Address>().unwrap();

            assert_eq!(expected, from.create2(salt, init_code_hash));
            assert_eq!(expected, from.create2_from_code(salt, init_code));
        }
    }
}
