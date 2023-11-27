use crate::{EvmPlatformSDK, SDK};

extern "C" {
    fn _evm_sload(key_ptr: *const u8, value_ptr: *mut u8);
    fn _evm_sstore(key_ptr: *const u8, value_ptr: *const u8);
}

impl EvmPlatformSDK for SDK {
    fn evm_sload(key: &[u8], value: &mut [u8]) {
        assert_eq!(key.len(), 32);
        assert_eq!(value.len(), 32);
        unsafe {
            _evm_sload(key.as_ptr(), value.as_mut_ptr());
        }
    }

    fn evm_sstore(key: &[u8], value: &[u8]) {
        assert_eq!(key.len(), 32);
        assert_eq!(value.len(), 32);
        unsafe {
            _evm_sstore(key.as_ptr(), value.as_ptr());
        }
    }
}
