use core::ptr;
use fluentbase_sdk::evm::ExecutionContext;

#[no_mangle]
pub fn _evm_address(output20_offset: *mut u8) {
    let address = ExecutionContext::contract_address();
    unsafe { ptr::copy(address.as_ptr(), output20_offset, 20) };
}
