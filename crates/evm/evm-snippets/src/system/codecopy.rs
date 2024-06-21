#[no_mangle]
pub fn system_codecopy() {
    // let dest_offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    // let offset = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    // let size = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    //
    // let dest_offset = u256_be_to_u64tuple_le(dest_offset);
    // let offset = u256_be_to_u64tuple_le(offset);
    // let size = u256_be_to_u64tuple_le(size);
    //
    // let (contract_bytecode_offset, contract_bytecode_length) = {
    //     let mut header = [0u8; 8];
    //     LowLevelSDK::read(
    //         header.as_mut_ptr(),
    //         header.len() as u32,
    //         <ContractInput as IContractInput>::ContractBytecode::FIELD_OFFSET as u32,
    //     );
    //     let offset = LittleEndian::read_u32(&header[0..4]);
    //     let length = LittleEndian::read_u32(&header[4..8]);
    //     (offset as usize, length as usize)
    // };
    //
    // // TODO validate inputs
    // // TODO rewrite using low level encoding to reduce result wasm size
    //
    // let dest_offset = dest_offset.0 as usize;
    // let offset = offset.0 as usize;
    // let size = size.0 as usize;
    //
    // let mut dest_data = unsafe { slice::from_raw_parts_mut(dest_offset as *mut u8, size) };
    //
    // let offset_tail_expected = offset + size;
    // let offset_tail_fact = if offset_tail_expected > contract_bytecode_length {
    //     contract_bytecode_length
    // } else {
    //     offset_tail_expected
    // };
    // if offset_tail_fact > offset {
    //     LowLevelSDK::read(
    //         dest_data.as_mut_ptr(),
    //         offset_tail_fact as u32 - offset as u32,
    //         (contract_bytecode_offset + offset) as u32,
    //     );
    // };
    // if offset_tail_fact < offset_tail_expected {
    //     dest_data[offset_tail_fact - offset..].fill(0);
    // }
}
