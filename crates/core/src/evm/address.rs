use crate::helpers::read_address_from_input;
use core::ptr;
use fluentbase_sdk::evm::{ContractInput, IContractInput};

#[no_mangle]
pub fn _evm_address(output20_offset: *mut u8) {
    let address =
        read_address_from_input(<ContractInput as IContractInput>::ContractAddress::FIELD_OFFSET);
    unsafe { ptr::copy(address.as_ptr(), output20_offset, 20) };
}
