use crate::{account_types::JZKT_ACCOUNT_BALANCE_FIELD, evm::read_address_from_input};
use core::ptr;
use fluentbase_sdk::{
    evm::{ContractInput, IContractInput},
    Bytes32,
    LowLevelAPI,
    LowLevelSDK,
};

#[no_mangle]
pub fn _evm_self_balance(output32_offset: *mut u8) {
    let mut bytes32 = Bytes32::default();
    let address =
        read_address_from_input(<ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET);
    unsafe { ptr::copy(address.as_ptr(), bytes32[12..].as_mut_ptr(), 20) }

    let _is_cold = LowLevelSDK::jzkt_get(
        bytes32.as_ptr(),
        JZKT_ACCOUNT_BALANCE_FIELD,
        output32_offset,
    );
}
