use crate::account_types::Topics;
use core::ptr;
use fluentbase_sdk::evm::ExecutionContext;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

pub fn _evm_log2(
    data_offset: *const u8,
    data_size: u32,
    topic32_1_offset: *const u8,
    topic32_2_offset: *const u8,
) {
    const TOPICS_COUNT: usize = 2;

    let address = ExecutionContext::contract_address();

    let mut topics = Topics::<TOPICS_COUNT>::default();
    unsafe { ptr::copy(topic32_1_offset, topics[0].as_mut_ptr(), 1) }
    unsafe { ptr::copy(topic32_2_offset, topics[1].as_mut_ptr(), 1) }

    LowLevelSDK::jzkt_emit_log(
        address.as_ptr(),
        topics.as_ptr(),
        topics.len() as u32 * 32,
        data_offset,
        data_size,
    );
}
