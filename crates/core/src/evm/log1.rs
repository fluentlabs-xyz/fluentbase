use crate::account_types::Topics;
use core::ptr;
use fluentbase_sdk::evm::ExecutionContext;
use fluentbase_sdk::{Bytes32, LowLevelAPI, LowLevelSDK};

pub fn _evm_log1(data_offset: *const u8, data_size: u32, topic32_1_offset: *const u8) {
    const TOPICS_COUNT: usize = 1;

    let mut address_bytes32 = Bytes32::default();
    let address = ExecutionContext::contract_address();
    unsafe { ptr::copy(address.as_ptr(), address_bytes32[12..].as_mut_ptr(), 20) };

    let mut topics = Topics::<TOPICS_COUNT>::default();
    unsafe { ptr::copy(topic32_1_offset, topics[0].as_mut_ptr(), 1) }

    LowLevelSDK::jzkt_emit_log(
        address_bytes32.as_ptr(),
        topics.as_ptr(),
        topics.len() as u32,
        data_offset,
        data_size,
    );
}
