use crate::evm::sload::_evm_sload;
use core::ptr;
use fluentbase_sdk::{Bytes32, LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;

#[no_mangle]
pub fn _evm_sstore(
    address20_offset: *const u8,
    index32_offset: *const u8,
    value32_offset: *const u8,
    previous_or_original_value32_offset: *mut u8,
    present_value32_offset: *mut u8,
    new_value32_offset: *mut u8,
    is_cold_offset: *mut u32,
) -> ExitCode {
    let mut present_slot_value32 = Bytes32::default();
    let mut is_cold: u32 = 0;
    let sload_exit_code = _evm_sload(
        address20_offset,
        index32_offset,
        present_slot_value32.as_mut_ptr(),
        is_cold as *mut u32,
    );
    if sload_exit_code == ExitCode::Ok {
        let mut slot_value32 = Bytes32::default();
        let _slot_value32_load_res =
            LowLevelSDK::jzkt_load(index32_offset, slot_value32.as_mut_ptr());
        // new value is same as present, we don't need to do anything
        let mut new_value32 = Bytes32::default();
        unsafe { ptr::copy(value32_offset, new_value32.as_mut_ptr(), 32) }

        if present_slot_value32 != new_value32 {
            LowLevelSDK::jzkt_store(index32_offset, new_value32.as_ptr());
        }

        unsafe {
            ptr::copy(
                slot_value32.as_ptr(),
                previous_or_original_value32_offset,
                32,
            )
        };
        unsafe { ptr::copy(present_slot_value32.as_ptr(), present_value32_offset, 32) };
        unsafe { ptr::copy(new_value32.as_ptr(), new_value32_offset, 32) };
        unsafe { *is_cold_offset = is_cold };

        return ExitCode::Ok;
    }
    return ExitCode::EVMNotFound;
}
