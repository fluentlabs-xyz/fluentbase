use core::slice;
use fluentbase_sdk::{evm::ExecutionContext, LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;

#[no_mangle]
pub fn _evm_revert(output_offset: *mut u8, size: u32) {
    if size <= 0 {
        return;
    }
    let mem_chunk = unsafe { slice::from_raw_parts(output_offset as *const u8, size as usize) };
    let checkpoint = ExecutionContext::journal_checkpoint();
    LowLevelSDK::jzkt_rollback(checkpoint);
    LowLevelSDK::sys_write(mem_chunk);
    LowLevelSDK::sys_halt(ExitCode::EVMCallRevert.into_i32());
}
