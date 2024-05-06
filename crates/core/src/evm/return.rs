use core::slice;
use fluentbase_sdk::{ContextReader, LowLevelAPI, LowLevelSDK};

pub fn _evm_return<CR: ContextReader>(output_offset: *mut u8, size: u32) {
    if size <= 0 {
        return;
    }

    let mem_chunk = unsafe { slice::from_raw_parts(output_offset as *const u8, size as usize) };
    LowLevelSDK::sys_write(mem_chunk);
}
