use fluentbase_sdk::{AccountManager, ContextReader};

pub fn _evm_log2<CR: ContextReader, AM: AccountManager>(
    _cr: &CR,
    _am: &AM,
    _data_offset: *const u8,
    _data_size: u32,
    _topic32_1_offset: *const u8,
    _topic32_2_offset: *const u8,
) {
    // const TOPICS_COUNT: usize = 2;
    // let address = cr.contract_address();
    // let mut topics = Topics::<TOPICS_COUNT>::default();
    // unsafe { ptr::copy(topic32_1_offset, topics[0].as_mut_ptr(), 1) }
    // unsafe { ptr::copy(topic32_2_offset, topics[1].as_mut_ptr(), 1) }
    // LowLevelSDK::jzkt_emit_log(
    //     address.as_ptr(),
    //     topics.as_ptr(),
    //     topics.len() as u32 * 32,
    //     data_offset,
    //     data_size,
    // );
}
