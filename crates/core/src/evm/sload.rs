use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;

#[no_mangle]
pub fn _evm_sload(
    _address20_offset: *const u8,
    index32_offset: *const u8,
    slot_value32_offset: *mut u8,
    is_cold: *mut u32,
) -> ExitCode {
    let slot_value32_load_res = LowLevelSDK::jzkt_load(index32_offset, slot_value32_offset);
    if slot_value32_load_res == -1 {
        return ExitCode::EVMNotFound;
    }

    unsafe { *is_cold = (slot_value32_load_res != 0) as u32 };

    ExitCode::Ok
}
