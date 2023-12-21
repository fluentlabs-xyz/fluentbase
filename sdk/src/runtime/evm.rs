use crate::{EvmPlatformSDK, SDK};
use alloy_primitives::{Address, U256};
use std::collections::HashMap;

#[derive(Default)]
struct EvmContext {
    storage: HashMap<[u8; 32], [u8; 32]>,
}

lazy_static::lazy_static! {
    static ref CONTEXT: std::sync::Mutex<EvmContext> = std::sync::Mutex::new(EvmContext::default());
}

const EMPTY_SLOT: [u8; 32] = [0; 32];

impl EvmPlatformSDK for SDK {
    fn evm_sload(key: &[u8], value: &mut [u8]) {
        if let Some(slot) = CONTEXT.lock().unwrap().storage.get(key) {
            value.copy_from_slice(slot);
        } else {
            value.copy_from_slice(&EMPTY_SLOT);
        }
    }

    fn evm_sstore(key: &[u8], value: &[u8]) {
        let mut key32: [u8; 32] = [0; 32];
        key32.copy_from_slice(key);
        let mut value32: [u8; 32] = [0; 32];
        value32.copy_from_slice(value);
        CONTEXT.lock().unwrap().storage.insert(key32, value32);
    }
}

#[cfg(test)]
mod test {
    use crate::{EvmPlatformSDK, SDK};
    use hex_literal::hex;

    #[test]
    pub fn test_total_supply() {
        let key = hex!("0000000000000000000000000000000000000000000000000000000000000001");
        let value = hex!("0000000000000000000000000000000000000000000000000000000000000002");
        SDK::evm_sstore(&key, &value);
        let mut loaded_value: [u8; 32] = [0; 32];
        SDK::evm_sload(&key, &mut loaded_value);
        assert_eq!(value, loaded_value);
    }
}
