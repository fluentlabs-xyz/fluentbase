pub mod address;
pub mod balance;
pub mod call;
pub mod calldatacopy;
pub mod calldataload;
pub mod calldatasize;
pub mod codecopy;
pub mod codehash;
pub mod codesize;
pub mod create;
pub mod create2;
pub mod extcodecopy;
pub mod extcodehash;
pub mod extcodesize;
pub mod log0;
pub mod log1;
pub mod log2;
pub mod log3;
pub mod log4;
pub mod r#return;
pub mod revert;
pub mod selfbalance;
pub mod sload;
pub mod sstore;

use crate::account_types::JZKT_ACCOUNT_BALANCE_FIELD;
use byteorder::{ByteOrder, LittleEndian};
use core::ptr;
use fluentbase_sdk::{
    evm::{Address, ContractInput, IContractInput, B256, U256},
    Bytes32,
    LowLevelAPI,
    LowLevelSDK,
};
use revm_interpreter::primitives::ShanghaiSpec;

pub type SpecDefault = ShanghaiSpec;

#[inline]
pub(crate) fn get_contract_input_offset_and_len() -> (u32, u32) {
    let mut header = [0u8; 8];
    LowLevelSDK::sys_read(
        &mut header,
        <ContractInput as IContractInput>::ContractInput::FIELD_OFFSET as u32,
    );
    let offset = LittleEndian::read_u32(&header[0..4]);
    let length = LittleEndian::read_u32(&header[4..8]);
    (offset, length)
}

#[inline(always)]
fn read_address_from_input(offset: usize) -> Address {
    let mut address = [0u8; Address::len_bytes()];
    LowLevelSDK::sys_read(&mut address, offset as u32);
    Address::from(address)
}

#[inline(always)]
fn read_balance(address: Address, value: &mut U256) {
    let mut bytes32 = Bytes32::default();
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::jzkt_get(bytes32.as_ptr(), JZKT_ACCOUNT_BALANCE_FIELD, unsafe {
        value.as_le_slice_mut().as_mut_ptr()
    });
}

#[inline(always)]
pub fn calc_create_address(deployer: &Address, nonce: u64) -> Address {
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
pub fn calc_create2_address(deployer: &Address, salt: &B256, init_code_hash: &B256) -> Address {
    let mut bytes = [0; 85];
    bytes[0] = 0xff;
    bytes[1..21].copy_from_slice(deployer.as_slice());
    bytes[21..53].copy_from_slice(salt.as_slice());
    bytes[53..85].copy_from_slice(init_code_hash.as_slice());
    LowLevelSDK::crypto_keccak256(bytes.as_ptr(), bytes.len() as u32, bytes.as_mut_ptr());
    let bytes32: [u8; 32] = bytes[0..32].try_into().unwrap();
    Address::from_word(B256::from(bytes32))
}
