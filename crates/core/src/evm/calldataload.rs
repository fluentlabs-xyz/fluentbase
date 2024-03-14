use crate::helpers::get_contract_input_offset_and_len;
use core::ptr;
use fluentbase_sdk::{Bytes32, LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn _evm_calldataload(calldata_idx: u32, output32_offset: *mut u8) {
    let (calldata_offset, calldata_length) = get_contract_input_offset_and_len();
    let value: Bytes32 = if calldata_idx < calldata_length {
        let length = core::cmp::min(calldata_length - calldata_idx, 32) as usize;
        let mut value = Bytes32::default();
        if length > 0 {
            LowLevelSDK::sys_read(&mut value[..length], calldata_offset + calldata_idx);
        }
        value
    } else {
        Bytes32::default()
    };
    unsafe { ptr::copy(value.as_ptr(), output32_offset, 32) }
}
