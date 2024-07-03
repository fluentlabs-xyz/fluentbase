#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    codec::Codec,
    derive::solidity_storage,
    Address,
    Bytes,
    LowLevelSDK,
    SharedAPI,
    U256,
};

#[derive(Codec, Debug, Default, Clone, PartialEq)]
pub struct MyStruct {
    pub a: U256,
    pub b: U256,
    pub c: Bytes,
    pub d: Bytes,
}

solidity_storage! {
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
    U256[] Arr;
    Address[][][] NestedArr;
    Address Owner;
    Bytes Data;
    MyStruct SomeStruct;
    mapping(Address => MyStruct) MyStructMap;
}

#[cfg(test)]
mod test {
    use super::*;
    use fluentbase_sdk::{codec::Encoder, Address, Bytes, ContractInput, LowLevelSDK, U256};
    use hex_literal::hex;
    use serial_test::serial;

    fn with_test_input<T: Into<Bytes>>(input: T, caller: Option<Address>) {
        let mut contract_input = ContractInput::default();
        contract_input.contract_caller = caller.unwrap_or_default();

        LowLevelSDK::with_test_context(contract_input.encode_to_vec(0));
        let input: Bytes = input.into();
        LowLevelSDK::with_test_input(input.into());
    }

    #[serial]
    #[test]
    pub fn test_primitive_storage_dynamic_bytes() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let b = fluentbase_sdk::Bytes::from(
            "this is a really long string. this is a really long
    string. this is a really long string. this it really long string.",
        );
        let storage = Data::default();
        storage.set(b.clone());

        let result = storage.get();
        assert_eq!(result, b);
    }
    #[serial]
    #[test]
    pub fn test_nested_arr() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = NestedArr::default();

        let idx1 = U256::from(0);
        let idx2 = U256::from(0);
        let idx3 = U256::from(0);
        let value = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        storage.set(idx1, idx2, idx3, value);

        let result = storage.get(idx1, idx2, idx3);
        assert_eq!(result, value);
    }
    #[serial]
    #[test]
    pub fn test_storage_mapping() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = Balance::default();

        let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        let value: U256 = U256::from_str_radix("1000000000000000000", 10).unwrap(); // 1000

        storage.set(addr, value);

        let result = storage.get(addr);

        assert_eq!(result, value);
    }
    #[serial]
    #[test]
    pub fn test_storage_mapping_nested() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = Allowance::default();

        let addr1 = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        let addr2 = Address::from(hex!("70997970C51812dc3A010C7d01b50e0d17dc79C8"));

        let value: U256 = U256::from_str_radix("1000000000000000000", 10).unwrap();

        storage.set(addr1, addr2, value);

        let result = storage.get(addr1, addr2);

        assert_eq!(result, result);
    }
    #[serial]
    #[test]
    pub fn test_storage_primitive_address() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = Owner::default();

        let addr1 = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        storage.set(addr1);

        let result = storage.get();

        assert_eq!(result, result);
    }
    #[serial]
    #[test]
    pub fn test_storage_primitive_struct() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = SomeStruct::default();

        let a = U256::from(1);
        let b = U256::from(2);
        let c = fluentbase_sdk::Bytes::from(
            "this it really long string. this it really long
    string. this it really long string. this it really long string.",
        );
        let d = fluentbase_sdk::Bytes::from("short");

        let my_struct = MyStruct { a, b, c, d };

        storage.set(my_struct.clone());

        let result = storage.get();

        assert_eq!(result, my_struct);
    }

    #[serial]
    #[test]
    pub fn test_storage_mapping_struct() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = MyStructMap::default();

        let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        let a = U256::from(1);
        let b = U256::from(2);
        let c = fluentbase_sdk::Bytes::from(
            "this it really long string. this it really long
    string. this it really long string. this it really long string.",
        );
        let d = fluentbase_sdk::Bytes::from("short");

        let my_struct = MyStruct { a, b, c, d };

        storage.set(addr, my_struct.clone());

        let result = storage.get(addr);

        assert_eq!(result, my_struct.clone());
    }
}
