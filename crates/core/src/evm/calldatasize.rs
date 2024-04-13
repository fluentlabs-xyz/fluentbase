use byteorder::{ByteOrder, LittleEndian};
use core::ptr;
use fluentbase_sdk::{
    evm::{ContractInput, IContractInput},
    LowLevelAPI, LowLevelSDK,
};

pub fn _evm_calldatasize(output32_offset: *mut u8) {
    let calldata_len = {
        let mut header = [0u8; 8];
        LowLevelSDK::sys_read(
            &mut header,
            <ContractInput as IContractInput>::ContractInput::FIELD_OFFSET as u32,
        );
        LittleEndian::read_u32(&header[4..8])
    };

    unsafe {
        ptr::copy(
            calldata_len.to_be_bytes().as_ptr(),
            output32_offset.offset(32 - core::mem::size_of::<u32>() as isize),
            core::mem::size_of::<u32>(),
        )
    }
    // alternative solution, more flexible
    // unsafe {
    //     ptr::copy(
    //         ExecutionContext::contract_code_size()
    //             .to_be_bytes()
    //             .as_ptr(),
    //         output32_offset.offset(32 - core::mem::size_of::<u32>() as isize),
    //         core::mem::size_of::<u32>(),
    //     )
    // }
}
