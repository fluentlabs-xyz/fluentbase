use fluentbase_sdk::evm::ExecutionContext;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

pub fn _evm_log0(data_offset: *const u8, data_size: u32) {
    let address = ExecutionContext::contract_address();
    LowLevelSDK::jzkt_emit_log(
        address.as_ptr(),
        core::ptr::null(),
        0,
        data_offset,
        data_size,
    );
}
