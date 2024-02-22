use crate::{account::JZKT_ACCOUNT_BALANCE_FIELD, evm::read_address_from_input};
use core::ptr;
use fluentbase_sdk::{
    evm::{ContractInput, IContractInput},
    LowLevelAPI,
    LowLevelSDK,
};

#[no_mangle]
pub fn _evm_self_balance(output32_offset: *mut u8) {
    let mut bytes32 = [0u8; 32];
    let address =
        read_address_from_input(<ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET);
    unsafe {
        ptr::copy(address.as_ptr(), bytes32.as_mut_ptr(), 20);
    }
    LowLevelSDK::jzkt_get(
        bytes32.as_ptr(),
        JZKT_ACCOUNT_BALANCE_FIELD,
        output32_offset,
    );
}
