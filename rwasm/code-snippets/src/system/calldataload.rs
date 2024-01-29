use crate::{
    common::u256_be_to_u64tuple_le,
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_sdk::{
    evm::{ContractInput, IContractInput},
    LowLevelAPI,
    LowLevelSDK,
};

#[no_mangle]
fn system_calldataload() {
    let index = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let index = u256_be_to_u64tuple_le(index).0 as usize;
    let (offset, length) = {
        let mut header = [0u8; 8];
        LowLevelSDK::sys_read(
            &mut header,
            <ContractInput as IContractInput>::ContractInput::FIELD_OFFSET as u32,
        );
        let offset = LittleEndian::read_u32(&header[0..4]);
        let length = LittleEndian::read_u32(&header[4..8]);
        (offset, length)
    };
    let value: [u8; U256_BYTES_COUNT as usize] = if index < length as usize {
        let length = core::cmp::min(length - index as u32, U256_BYTES_COUNT as u32) as usize;
        let mut value = [0u8; U256_BYTES_COUNT as usize];
        if length > 0 {
            LowLevelSDK::sys_read(&mut value[..length], offset + index as u32);
        }
        value
    } else {
        [0u8; U256_BYTES_COUNT as usize]
    };
    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, value);
}
