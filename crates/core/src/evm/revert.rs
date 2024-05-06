use core::slice;
use fluentbase_sdk::{ContextReader, LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;

pub fn _evm_revert<CR: ContextReader>(output_offset: *mut u8, size: u32) {
    if size <= 0 {
        return;
    }
    let mem_chunk = unsafe { slice::from_raw_parts(output_offset as *const u8, size as usize) };
    let checkpoint = CR::journal_checkpoint();
    LowLevelSDK::jzkt_rollback(checkpoint);
    LowLevelSDK::sys_write(mem_chunk);
    LowLevelSDK::sys_halt(ExitCode::EVMCallRevert.into_i32());
}
