use fluentbase_sdk::{ContextReader, LowLevelAPI, LowLevelSDK};

pub fn _evm_log0<CR: ContextReader>(cr: &CR, data_offset: *const u8, data_size: u32) {
    let address = cr.contract_address();
    LowLevelSDK::jzkt_emit_log(
        address.as_ptr(),
        core::ptr::null(),
        0,
        data_offset,
        data_size,
    );
}
