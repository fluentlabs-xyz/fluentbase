#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use fluentbase_sdk::{codec::Codec, derive::solidity_storage, Address, Bytes, U256};

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
    use fluentbase_sdk::{
        journal::{JournalState, JournalStateBuilder},
        runtime::TestingContext,
    };
    use fluentbase_types::ContractContext;
    use hex_literal::hex;
    use serial_test::serial;

    fn with_test_input<T: Into<Vec<u8>>>(
        input: T,
        caller: Option<Address>,
    ) -> JournalState<TestingContext> {
        JournalStateBuilder::default()
            .with_contract_context(ContractContext {
                caller: caller.unwrap_or_default(),
                ..Default::default()
            })
            .with_devnet_genesis()
            .build(TestingContext::new().with_input(input))
    }

    #[serial]
    #[test]
    pub fn test_primitive_storage_dynamic_bytes() {
        let mut sdk = with_test_input(vec![], None);
        let mut storage = Data::new(&mut sdk);

        let b = fluentbase_sdk::Bytes::from(
            "this is a really long string. this is a really long
    string. this is a really long string. this it really long string.",
        );
        storage.set(b.clone());

        let result = storage.get();
        assert_eq!(result, b);
    }

    #[serial]
    #[test]
    pub fn test_nested_arr() {
        let mut sdk = with_test_input(vec![], None);
        let mut storage = NestedArr::new(&mut sdk);

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
        let mut sdk = with_test_input(vec![], None);
        let mut storage = Balance::new(&mut sdk);

        let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        let value: U256 = U256::from_str_radix("1000000000000000000", 10).unwrap(); // 1000

        storage.set(addr, value);

        let result = storage.get(addr);

        assert_eq!(result, value);
    }

    #[serial]
    #[test]
    pub fn test_storage_mapping_nested() {
        let mut sdk = with_test_input(vec![], None);
        let mut storage = Allowance::new(&mut sdk);

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
        let mut sdk = with_test_input(vec![], None);
        let mut storage = Owner::new(&mut sdk);

        let addr1 = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        storage.set(addr1);

        let result = storage.get();

        assert_eq!(result, result);
    }

    #[serial]
    #[test]
    pub fn test_storage_primitive_struct() {
        let mut sdk = with_test_input(vec![], None);
        let mut storage = SomeStruct::new(&mut sdk);

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
        let mut sdk = with_test_input(vec![], None);
        let mut storage = MyStructMap::new(&mut sdk);

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
