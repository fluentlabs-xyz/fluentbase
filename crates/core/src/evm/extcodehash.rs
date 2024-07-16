use fluentbase_sdk::{ContextReader, SovereignAPI};

pub fn _evm_extcodehash<CR: ContextReader, AM: SovereignAPI>(
    _cr: &CR,
    _am: &AM,
    _address20_offset: *mut u8,
    _output32_offset: *mut u8,
) {
    // let mut address_bytes32 = Bytes32::default();
    // unsafe { ptr::copy(address20_offset, address_bytes32[12..].as_mut_ptr(), 20) }
    // let _is_cold = LowLevelSDK::jzkt_get(
    //     address_bytes32.as_ptr(),
    //     JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    //     output32_offset,
    // );
}
