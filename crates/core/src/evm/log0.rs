use core::{ptr, ptr::null};
use fluentbase_sdk::evm::ExecutionContext;
use fluentbase_sdk::{Bytes32, LowLevelAPI, LowLevelSDK};

pub fn _evm_log0(data_offset: *const u8, data_size: u32) {
    let mut address_bytes32 = Bytes32::default();
    let address = ExecutionContext::contract_address();
    unsafe { ptr::copy(address.as_ptr(), address_bytes32[12..].as_mut_ptr(), 20) }

    LowLevelSDK::jzkt_emit_log(address_bytes32.as_ptr(), null(), 0, data_offset, data_size);
}
