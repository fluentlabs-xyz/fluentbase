use crate::evm::ZKTRIE_BALANCE_FIELD;
use core::ptr;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn _evm_balance(address20_offset: *const u8, output32_offset: *mut u8) {
    let mut bytes32 = [0u8; 32];
    unsafe { ptr::copy(address20_offset, bytes32.as_mut_ptr(), 20) }
    LowLevelSDK::jzkt_get(bytes32.as_ptr(), ZKTRIE_BALANCE_FIELD, output32_offset);
}
