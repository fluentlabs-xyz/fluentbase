#[no_mangle]
fn system_calldataload() {
    // let index = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    // let index = u256_be_to_u64tuple_le(index).0 as usize;
    // let (offset, length) = {
    //     let mut header = [0u8; 8];
    //     LowLevelSDK::read(
    //         header.as_mut_ptr(),
    //         header.len() as u32,
    //         <ContractInput as IContractInput>::ContractInput::FIELD_OFFSET as u32,
    //     );
    //     let offset = LittleEndian::read_u32(&header[0..4]);
    //     let length = LittleEndian::read_u32(&header[4..8]);
    //     (offset, length)
    // };
    // let value: [u8; U256_BYTES_COUNT as usize] = if index < length as usize {
    //     let length = core::cmp::min(length - index as u32, U256_BYTES_COUNT as u32) as usize;
    //     let mut value = u256_zero();
    //     if length > 0 {
    //         LowLevelSDK::read(value.as_mut_ptr(), length as u32, offset + index as u32);
    //     }
    //     value
    // } else {
    //     u256_zero()
    // };
    // stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, value);
}
