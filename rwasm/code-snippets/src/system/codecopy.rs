use crate::{
    common::u256_be_to_u64tuple_le,
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
};
use byteorder::{ByteOrder, LittleEndian};
use core::slice;
use fluentbase_sdk::{
    evm::{ContractInput, IContractInput},
    LowLevelAPI,
    LowLevelSDK,
};

#[no_mangle]
pub fn system_codecopy() {
    let dest_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let dest_offset = u256_be_to_u64tuple_le(dest_offset);
    let offset = u256_be_to_u64tuple_le(offset);
    let size = u256_be_to_u64tuple_le(size);

    let (contract_bytecode_offset, contract_bytecode_length) = {
        let mut header = [0u8; 8];
        LowLevelSDK::sys_read(
            &mut header,
            <ContractInput as IContractInput>::ContractBytecode::FIELD_OFFSET as u32,
        );
        let offset = LittleEndian::read_u32(&header[0..4]);
        let length = LittleEndian::read_u32(&header[4..8]);
        (offset as usize, length as usize)
    };

    // TODO validate inputs
    // TODO rewrite using low level encoding to reduce result wasm size

    let dest_offset = dest_offset.0 as usize;
    let offset = offset.0 as usize;
    let size = size.0 as usize;

    let mut dest_data = unsafe { slice::from_raw_parts_mut(dest_offset as *mut u8, size) };

    let offset_tail_expected = offset + size;
    let offset_tail_fact = if offset_tail_expected > contract_bytecode_length {
        contract_bytecode_length
    } else {
        offset_tail_expected
    };
    if offset_tail_fact > offset {
        LowLevelSDK::sys_read(
            &mut dest_data[0..offset_tail_fact - offset],
            (contract_bytecode_offset + offset) as u32,
        );
    };
    if offset_tail_fact < offset_tail_expected {
        dest_data[offset_tail_fact - offset..].fill(0);
    }
}
