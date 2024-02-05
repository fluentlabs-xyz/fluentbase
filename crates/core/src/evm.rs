mod balance;
mod create;
mod selfbalance;

use alloc::vec::Vec;
use byteorder::{BigEndian, ByteOrder};
use core::ptr;
use fluentbase_sdk::{
    evm::{Address, Bytes, B256, U256},
    Bytes32,
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{KECCAK_EMPTY, POSEIDON_EMPTY};
use revm_interpreter::{
    primitives::{Bytecode, Env, HashMap},
    CallInputs,
    CreateInputs,
    Gas,
    Host,
    InstructionResult,
    SelfDestructResult,
    SharedMemory,
};

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

#[derive(Default)]
pub(crate) struct HostImpl {
    accounts: HashMap<Address, Account>,
    logs: Vec<(Address, Vec<B256>, Bytes)>,
}

impl Host for HostImpl {
    fn env(&mut self) -> &mut Env {
        todo!()
    }

    fn load_account(&mut self, address: Address) -> Option<(bool, bool)> {
        let value = self.accounts.get(&address);
        if value.is_some() {
            return Some((false, true));
        }
        self.accounts
            .insert(address, Account::read_account(&address));
        Some((true, true))
    }

    fn block_hash(&mut self, _number: U256) -> Option<B256> {
        todo!("not supported opcode")
    }

    fn balance(&mut self, address: Address) -> Option<(U256, bool)> {
        let (is_cold, exist) = self.load_account(address)?;
        if !exist {
            return Some((U256::ZERO, exist));
        }
        let Account { balance, .. } = self.accounts.get(&address).unwrap();
        Some((*balance, is_cold))
    }

    fn code(&mut self, address: Address) -> Option<(Bytecode, bool)> {
        todo!()
    }

    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        todo!()
    }

    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)> {
        todo!()
    }

    fn sstore(
        &mut self,
        address: Address,
        index: U256,
        value: U256,
    ) -> Option<(U256, U256, U256, bool)> {
        todo!()
    }

    fn tload(&mut self, _address: Address, _index: U256) -> U256 {
        todo!("not supported opcode")
    }

    fn tstore(&mut self, _address: Address, _index: U256, _value: U256) {
        todo!("not supported opcode")
    }

    fn log(&mut self, address: Address, topics: Vec<B256>, data: Bytes) {
        self.logs.push((address, topics, data));
    }

    fn call(
        &mut self,
        input: &mut CallInputs,
        shared_memory: &mut SharedMemory,
    ) -> (InstructionResult, Gas, Bytes) {
        todo!()
    }

    fn create(
        &mut self,
        inputs: &mut CreateInputs,
        shared_memory: &mut SharedMemory,
    ) -> (InstructionResult, Option<Address>, Gas, Bytes) {
        todo!()
    }

    fn selfdestruct(&mut self, _address: Address, _target: Address) -> Option<SelfDestructResult> {
        todo!("not supported opcode")
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
