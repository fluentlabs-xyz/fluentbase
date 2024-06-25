use alloy_sol_types::{sol, SolValue};
use fluentbase_sdk::{Address, LowLevelSDK, SharedAPI, U256};

use crate::AllowanceStorage;

pub trait IMappingStorage {
    fn storage_key(slot: U256, key: U256) -> U256;
}

struct MappingStorage {
    slot: U256,
}

impl IMappingStorage for MappingStorage {
    fn storage_key(slot: U256, key: U256) -> U256 {
        let mut raw_storage_key: [u8; 64] = [0; 64];
        raw_storage_key[0..32].copy_from_slice(slot.as_le_slice());
        raw_storage_key[32..64].copy_from_slice(key.as_le_slice());
        let mut storage_key: [u8; 32] = [0; 32];
        LowLevelSDK::keccak256(
            raw_storage_key.as_ptr(),
            raw_storage_key.len() as u32,
            storage_key.as_mut_ptr(),
        );
        U256::from_be_bytes(storage_key)
    }
}

impl MappingStorage {
    fn key(&self, arg1: Address, arg2: Address) -> U256 {
        let arg1_key = <MappingStorage as IMappingStorage>::storage_key(
            self.slot,
            U256::from_be_slice(arg1.abi_encode().as_slice()),
        );
        let arg2_key = <MappingStorage as IMappingStorage>::storage_key(
            arg1_key,
            U256::from_be_slice(arg2.abi_encode().as_slice()),
        );

        arg2_key
    }
}

// #[cfg(test)]
// mod tests {
//     use quote::quote;
// }

// // solidity_storage! {mapping(Address => mapping(Address => U256)) AllowanceStorage}

pub fn mapping_storage_key(slot: U256, key: U256) -> U256 {
    let mut raw_storage_key: [u8; 64] = [0; 64];
    raw_storage_key[0..32].copy_from_slice(slot.as_le_slice());
    raw_storage_key[32..64].copy_from_slice(key.as_le_slice());
    let mut storage_key: [u8; 32] = [0; 32];
    LowLevelSDK::keccak256(
        raw_storage_key.as_ptr(),
        raw_storage_key.len() as u32,
        storage_key.as_mut_ptr(),
    );
    U256::from_be_bytes(storage_key)
}

pub trait StorageKey {
    fn key(&self, args: Vec<U256>) -> U256;
}
#[macro_export]
macro_rules! solidity_storage_mapping {
    // Базовый случай: один уровень вложенности
    (mapping($key1:ty => $value:ty) $name:ident;) => {
        pub struct $name {
            slot: U256,
        }

        impl $name {
            pub fn new(slot: U256) -> Self {
                Self { slot }
            }
        }

        impl StorageKey for $name {
            fn key(&self, arg1: $key1) -> U256 {
                mapping_storage_key(self.slot, U256::from_be_slice(arg1.abi_encode().as_slice()))
            }
        }
    };

    // Рекурсивный случай: вложенный маппинг
    (mapping($key1:ty => mapping($key2:ty => $value:ty)) $name:ident;) => {
        pub struct $name {
            slot: U256,
        }

        impl $name {
            pub fn new(slot: U256) -> Self {
                Self { slot }
            }
        }

        impl StorageKey for $name {
            fn key(&self, arg1: $key1, arg2: $key2) -> U256 {
                let arg1_key = mapping_storage_key(
                    self.slot,
                    U256::from_be_slice(arg1.abi_encode().as_slice()),
                );
                mapping_storage_key(
                    arg1_key,
                    U256::from_be_slice(arg2.abi_encode().as_slice()),
                )
            }
        }
    };

    // Множественная вложенность: больше двух уровней вложенности
    (mapping($key1:ty => mapping($($key:ty =>)+ $value:ty)) $name:ident;) => {
        pub struct $name {
            slot: U256,
        }

        impl $name {
            pub fn new(slot: U256) -> Self {
                Self { slot }
            }
        }

        impl StorageKey for $name {
            fn key(&self, arg1: $key1, $($arg: $key),+) -> U256 {
                let mut current_key = mapping_storage_key(
                    self.slot,
                    U256::from_be_slice(arg1.abi_encode().as_slice()),
                );

                $(
                    current_key = mapping_storage_key(
                        current_key,
                        U256::from_be_slice($arg.abi_encode().as_slice()),
                    );
                )+

                current_key
            }
        }
    };
}

solidity_storage_mapping! {
    mapping(Address => mapping(Address => mapping(Address => U256))) AllowancesStorage;
}
