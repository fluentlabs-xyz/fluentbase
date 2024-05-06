use core::ptr;
use fluentbase_sdk::ContextReader;

pub fn _evm_address<CR: ContextReader>(output20_offset: *mut u8) {
    let address = CR::contract_address();
    unsafe { ptr::copy(address.as_ptr(), output20_offset, 20) };
}
