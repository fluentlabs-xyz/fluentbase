#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate fluentbase_sdk;

use alloy_sol_types::{sol, SolValue};
use fluentbase_sdk::{derive::solidity_storage, Address, LowLevelSDK, SharedAPI, U256};

solidity_storage! {
    U256[][] Arr;
    mapping(Address => mapping(Address => U256)) AllowanceStorage;
    mapping(Address owner => mapping(Address users => mapping(Address balances => MyStruct))) BalancesStorage;
}

sol! {
    contract HelloWorld {
        U256[] arr;
    }
}

pub fn mapping_key(slot: U256, key: U256) -> U256 {
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

pub fn array_key(slot: U256, index: U256) -> U256 {
    let mut storage_key: [u8; 32] = [0; 32];
    LowLevelSDK::keccak256(slot.as_le_slice().as_ptr(), 32, storage_key.as_mut_ptr());
    let storage_key = U256::from_be_bytes(storage_key);
    storage_key + index
}

pub fn field_key(slot: U256) -> U256 {
    slot
}

// Trait for saving data to storage:
pub trait Mapping {
    fn get(&self, key: fluentbase_sdk::U256) -> Self;
    fn set(&self, key: fluentbase_sdk::U256, value: Self);
}

pub trait Array {
    fn get(&self, index: fluentbase_sdk::U256) -> Self;
    fn set(&self, index: fluentbase_sdk::U256, value: Self);
    fn length(&self) -> fluentbase_sdk::U256;
}

pub trait DynamicArray: Array {
    fn push(&self, value: Self);
    fn pop(&self) -> Self;
}

pub trait Field {
    fn get(&self) -> Self;
    fn set(&self, value: Self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk::U256;

    #[test]
    fn test_mapping_storage() {
        let owner = Address::default();
        let spender = Address::default();
        let _amount = U256::from(100);

        let allowance_storage = AllowanceStorage {};
        print!("{:?}", AllowanceStorage::SLOT);

        let key = allowance_storage.key(owner, spender);
        println!("{:?}", key);

        let expected_key = U256::from_str_radix(
            "45674214084458039979679588399856379396083966111810618268439781217138252554320",
            10,
        )
        .expect("failed to parse U256 from string");
        assert_eq!(key, expected_key);
    }
}
