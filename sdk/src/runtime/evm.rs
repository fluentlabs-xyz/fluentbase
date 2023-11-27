use crate::{EvmPlatformSDK, SDK};

impl EvmPlatformSDK for SDK {
    fn evm_sload(key: &[u8], value: &mut [u8]) {
        unreachable!("its not possible here")
    }

    fn evm_sstore(key: &[u8], value: &[u8]) {
        unreachable!("its not possible here")
    }
}
