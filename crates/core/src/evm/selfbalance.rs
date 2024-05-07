use core::ptr;
use fluentbase_sdk::{AccountManager, ContextReader, LowLevelAPI, LowLevelSDK};
use fluentbase_types::Bytes32;

pub fn _evm_self_balance<CR: ContextReader, AM: AccountManager>(
    _cr: &CR,
    _am: &AM,
    _output32_offset: *mut u8,
) {
    // let mut bytes32 = Bytes32::default();
    // let address = cr.contract_address();
    // unsafe { ptr::copy(address.as_ptr(), bytes32[12..].as_mut_ptr(), 20) }
    // let _is_cold = LowLevelSDK::jzkt_get(
    //     bytes32.as_ptr(),
    //     JZKT_ACCOUNT_BALANCE_FIELD,
    //     output32_offset,
    // );
}
