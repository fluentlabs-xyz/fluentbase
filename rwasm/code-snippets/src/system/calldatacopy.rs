use crate::{
    common::u256_be_to_tuple_le,
    common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};
use byteorder::{ByteOrder, LittleEndian};
use core::slice;
use fluentbase_sdk::{
    evm::{ContractInput, IContractInput},
    LowLevelAPI,
    LowLevelSDK,
};

#[no_mangle]
fn system_calldatacopy() {
    let dest_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let src_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let mut dest_offset = u256_be_to_tuple_le(dest_offset).0 as usize;
    let mut src_offset = u256_be_to_tuple_le(src_offset).0 as usize;
    let size_left = u256_be_to_tuple_le(size);

    let (ci_offset, ci_length) = {
        let mut header = [0u8; 8];
        LowLevelSDK::sys_read(
            &mut header,
            <ContractInput as IContractInput>::ContractInput::FIELD_OFFSET as u32,
        );
        let offset = LittleEndian::read_u32(&header[0..4]);
        let length = LittleEndian::read_u32(&header[4..8]);
        (offset, length)
    };

    if size_left.0 <= 0 || size_left.1 > 0 || size_left.2 > 0 || size_left.3 > 0 {
        return;
    };
    let mut size_left = size_left.0 as usize;

    let dest = unsafe { slice::from_raw_parts_mut(dest_offset as *mut u8, size_left) };

    let mut shift = 0;
    while size_left > 0 {
        let ci_chunk_size_expected = core::cmp::min(U256_BYTES_COUNT as usize, size_left);

        let ci_length_left = if src_offset < ci_length as usize {
            ci_length as usize - src_offset
        } else {
            0
        };
        let ci_chunk_size = core::cmp::min(ci_chunk_size_expected, ci_length_left);

        let mut v = [0u8; U256_BYTES_COUNT as usize];
        if src_offset < ci_length as usize && ci_chunk_size > 0 {
            LowLevelSDK::sys_read(&mut v[..ci_chunk_size], ci_offset + src_offset as u32);
        };

        dest[shift..shift + ci_chunk_size].copy_from_slice(&v[0..ci_chunk_size]);

        shift += ci_chunk_size_expected;
        src_offset += shift;
        size_left -= ci_chunk_size_expected;
    }
}
