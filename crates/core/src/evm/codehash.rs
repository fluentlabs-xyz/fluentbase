use crate::account_types::JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD;
use core::ptr;
use fluentbase_sdk::{ContextReader, LowLevelAPI, LowLevelSDK};
use fluentbase_types::Bytes32;

pub fn _evm_codehash<CR: ContextReader>(cr: &CR, output32_offset: *mut u8) {
    let mut address_bytes32 = Bytes32::default();
    let address = cr.contract_address();
    unsafe { ptr::copy(address.as_ptr(), address_bytes32[12..].as_mut_ptr(), 20) }
    let _is_cold = LowLevelSDK::jzkt_get(
        address_bytes32.as_ptr(),
        JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
        output32_offset,
    );
}
