use fluentbase_sdk::{ContextReader, SovereignAPI};

pub fn _evm_log0<CR: ContextReader, AM: SovereignAPI>(
    _cr: &CR,
    _am: &AM,
    _data_offset: *const u8,
    _data_size: u32,
) {
    // let address = cr.contract_address();
    // LowLevelSDK::jzkt_emit_log(
    //     address.as_ptr(),
    //     core::ptr::null(),
    //     0,
    //     data_offset,
    //     data_size,
    // );
}
