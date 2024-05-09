use fluentbase_sdk::ContextReader;

pub fn _evm_log1<CR: ContextReader>(
    _cr: &CR,
    _data_offset: *const u8,
    _data_size: u32,
    _topic32_1_offset: *const u8,
) {
    // const TOPICS_COUNT: usize = 1;
    // let address = cr.contract_address();
    // LowLevelSDK::jzkt_emit_log(
    //     address.as_ptr(),
    //     topic32_1_offset as *const [u8; 32],
    //     32,
    //     data_offset,
    //     data_size,
    // );
}
