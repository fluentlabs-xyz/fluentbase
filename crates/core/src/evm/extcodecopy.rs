use crate::{account::Account, evm::read_address_from_input};
use alloc::vec;
use byteorder::{ByteOrder, LittleEndian};
use core::ptr;
use fluentbase_sdk::{
    evm::{ContractInput, IContractInput},
    Bytes32,
};

#[no_mangle]
pub fn _evm_extcodecopy(
    address20_offset: *const u8,
    output_offset: *mut u8,
    code_index: u32,
    len: u32,
) {
    let mut address_bytes32 = Bytes32::default();
    unsafe { ptr::copy(address20_offset, address_bytes32[12..].as_mut_ptr(), 20) }

    let mut source_code_hash32 = Bytes32::default();
    let mut source_code_size32 = Bytes32::default();
    Account::get_source_code_hash(address_bytes32.as_ptr(), source_code_hash32.as_mut_ptr());
    Account::get_source_code_size(address_bytes32.as_ptr(), source_code_size32.as_mut_ptr());
    let source_code_size = LittleEndian::read_u64(&source_code_size32);
    let mut bytecode = vec![0u8; source_code_size as usize];
    Account::copy_bytecode(source_code_hash32.as_ptr(), bytecode.as_mut_ptr());
    let bytecode_tail_idx = bytecode.len() as u32;
    let required_tail_idx = code_index + len;
    let min_tail_idx = core::cmp::min(bytecode_tail_idx, required_tail_idx);
    for i in code_index..min_tail_idx {
        unsafe { *output_offset.offset(i as isize) = bytecode[i as usize] };
    }
    for i in min_tail_idx..required_tail_idx {
        unsafe { *output_offset.offset(i as isize) = 0 };
    }
}
