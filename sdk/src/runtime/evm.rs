use crate::{EvmPlatformSDK, SDK};

impl EvmPlatformSDK for SDK {
    fn evm_sload(_key: &[u8], _value: &mut [u8]) {
        unreachable!("its not possible here")
    }

    fn evm_sstore(_key: &[u8], _value: &[u8]) {
        unreachable!("its not possible here")
    }
}
