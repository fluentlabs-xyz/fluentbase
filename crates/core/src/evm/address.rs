use core::ptr;
use fluentbase_sdk::ContextReader;

pub fn _evm_address<CR: ContextReader>(cr: &CR, output20_offset: *mut u8) {
    let address = cr.contract_address();
    unsafe { ptr::copy(address.as_ptr(), output20_offset, 20) };
}
