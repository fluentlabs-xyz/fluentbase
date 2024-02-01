#![no_std]

extern crate alloc;
extern crate fluentbase_sdk;

use alloc::alloc::alloc;
use byteorder::{BigEndian, ByteOrder};
use core::{alloc::Layout, ptr};
use fluentbase_sdk::{
    evm::{ContractInput, ExecutionContext, IContractInput},
    LowLevelAPI,
    LowLevelSDK,
};
use fluentbase_types::{Address, ExitCode, B256, KECCAK_EMPTY, POSEIDON_EMPTY, U256};

const ZKTRIE_CODESIZE_NONCE_FIELD: u32 = 1;
const ZKTRIE_BALANCE_FIELD: u32 = 2;
const ZKTRIE_ROOT_FIELD: u32 = 3;
const ZKTRIE_KECCAK_CODE_HASH_FIELD: u32 = 4;
const ZKTRIE_CODE_HASH_FIELD: u32 = 5;

struct Account {
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
    fn is_not_empty(&self) -> bool {
        self.nonce != 0 || self.keccak_code_hash != KECCAK_EMPTY || self.code_hash != POSEIDON_EMPTY
    }

    #[inline(always)]
    fn transfer_value(&mut self, to: &mut Self, value: &U256) -> bool {
        self.balance.checked_sub(*value).is_some() && to.balance.checked_add(*value).is_some()
    }
}

#[inline(always)]
fn _unsafe_read_input_address(offset: usize) -> Address {
    let mut address = [0u8; Address::len_bytes()];
    LowLevelSDK::sys_read(&mut address, offset as u32);
    Address::from(address)
}

#[inline(always)]
fn _unsafe_caller_balance(value: &mut U256) {
    let mut bytes32 = [0u8; 32];
    let address =
        _unsafe_read_input_address(<ContractInput as IContractInput>::ContractCaller::FIELD_OFFSET);
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, unsafe {
        value.as_le_slice_mut().as_mut_ptr()
    });
}

#[inline(always)]
fn _unsafe_callee_balance(value: &mut U256) {
    let mut bytes32 = [0u8; 32];
    let address = _unsafe_read_input_address(
        <ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET,
    );
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, unsafe {
        value.as_le_slice_mut().as_mut_ptr()
    });
}

#[inline(always)]
fn _unsafe_read_balance(address: Address, value: &mut U256) {
    let mut bytes32 = [0u8; 32];
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, unsafe {
        value.as_le_slice_mut().as_mut_ptr()
    });
}

#[inline(always)]
fn _unsafe_write_balance(address: Address, value: &mut U256) {
    let mut bytes32 = [0u8; 32];
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, unsafe {
        value.as_le_slice_mut().as_mut_ptr()
    });
}

#[inline(always)]
fn _unsafe_transfer_balance(from: &Account, to: &Account, value: &U256) -> bool {
    if from.balance.checked_sub(*value).is_none() {
        return false;
    } else if to.balance.checked_add(*value).is_none() {
        return false;
    }
    false
}

#[no_mangle]
pub fn _evm_self_balance(output32_offset: *mut u8) {
    let mut bytes32 = [0u8; 32];
    let address = _unsafe_read_input_address(
        <ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET,
    );
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, output32_offset);
}

#[no_mangle]
pub fn _evm_balance(address20_offset: *const u8, output32_offset: *mut u8) {
    let mut bytes32 = [0u8; 32];
    unsafe { ptr::copy(address20_offset, bytes32.as_mut_ptr(), 20) }
    LowLevelSDK::zktrie_field(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, output32_offset);
}

#[inline(always)]
fn _calc_create_address(nonce: u64, deployer: &Address) -> Address {
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

#[no_mangle]
pub fn _evm_create(
    value32_offset: *const u8,
    code_offset: *const u8,
    code_length: u32,
    output20_offset: *mut u8,
    gas_limit: u32,
) -> i32 {
    // check write protection
    if ExecutionContext::contract_is_static() {
        return ExitCode::WriteProtection.into_i32();
    }
    // read value input and contract address
    let value = U256::from_be_slice(unsafe { &*ptr::slice_from_raw_parts(value32_offset, 32) });
    let contract_address = _unsafe_read_input_address(
        <ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET,
    );
    // load deployer and contract accounts
    let mut deployer = Account::read_account(&contract_address);
    let created_address = _calc_create_address(deployer.nonce, &contract_address);
    let mut contract = Account::read_account(&created_address);
    // calc keccak256 and poseidon hashes for account
    LowLevelSDK::crypto_keccak256(
        code_offset,
        code_length,
        contract.keccak_code_hash.as_mut_ptr(),
    );
    LowLevelSDK::crypto_poseidon(code_offset, code_length, contract.code_hash.as_mut_ptr());
    // if nonce or code is not empty then its collision
    if contract.is_not_empty() {
        return ExitCode::CreateCollision.into_i32();
    }
    contract.nonce = 1;
    // transfer value to the just created account
    if deployer.transfer_value(&mut contract, &value) {
        return ExitCode::InsufficientBalance.into_i32();
    }
    // execute deployer bytecode
    LowLevelSDK::sys_exec(
        code_offset,
        code_length,
        ptr::null(),
        0,
        ptr::null_mut(),
        0,
        gas_limit,
    );
    let bytecode_length = LowLevelSDK::sys_output_size();
    let bytecode = unsafe {
        alloc(Layout::from_size_align_unchecked(
            bytecode_length as usize,
            1,
        ))
    };
    LowLevelSDK::sys_read_output(bytecode, 0, bytecode_length);
    ExitCode::Ok.into_i32()
}
