use core::slice;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn _evm_revert(output_offset: *mut u8, size: u32) {
    if size <= 0 {
        return;
    }

    let mem_chunk = unsafe { slice::from_raw_parts(output_offset as *const u8, size as usize) };
    let checkpoint = LowLevelSDK::jzkt_checkpoint();
    LowLevelSDK::jzkt_rollback(checkpoint.0, checkpoint.1);
    LowLevelSDK::sys_write(mem_chunk);
    LowLevelSDK::sys_halt(0);
}
