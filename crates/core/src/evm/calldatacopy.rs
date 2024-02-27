use crate::evm::get_calldata_input_offset_and_len;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn _evm_calldatacopy(calldata_idx: u32, len: u32, output32_offset: *mut u8) {
    if len <= 0 {
        return;
    }
    let (calldata_offset, calldata_len) = get_calldata_input_offset_and_len();
    let mut output = unsafe { core::slice::from_raw_parts_mut(output32_offset, len as usize) };
    if calldata_idx < calldata_len {
        let copy_len = core::cmp::min(calldata_len - calldata_idx, len) as usize;
        if copy_len > 0 {
            LowLevelSDK::sys_read(&mut output[..copy_len], calldata_offset + calldata_idx);
        }
        if copy_len < len as usize {
            output[copy_len..len as usize].fill(0);
        }
    } else {
        output.fill(0);
    }
}
