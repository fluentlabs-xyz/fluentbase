#![cfg_attr(target_arch = "wasm32", no_std, no_main)]
use fluentbase_sdk::{
    codec::Codec,
    derive::solidity_storage,
    entrypoint,
    Address,
    Bytes,
    ExitCode,
    FixedBytes,
    SharedAPI,
    I256,
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
    // For this types DirectStorage is used (it's allows to reduce code size by avoiding redundant encoding/decoding)
    FixedBytes<32> CustomFixedBytes;
    [u8; 32] CustomFixedBytesArray;
    U256 CustomU256;
    I256 CustomI256;
    u64 CustomU64;
    i64 CustomI64;

    // For this types SolidityStorage is used (it's allows to use dynamic types like Vec<u8> or String)
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
    U256[] Arr;
    Address[][][] NestedArr;
    Address Owner;
    Bytes Data;
    MyStruct SomeStruct;
    mapping(Address => MyStruct) MyStructMap;
}

pub fn main_entry(sdk: impl SharedAPI) {
    sdk.exit();
}

entrypoint!(main_entry);

#[cfg(test)]
mod test {
    use super::*;
    use fluentbase_sdk::{hex, ContractContextV1};
    use fluentbase_sdk_testing::HostTestingContext;
    use serial_test::serial;

    fn with_test_input<T: Into<Bytes>>(input: T, caller: Option<Address>) -> HostTestingContext {
        HostTestingContext::default()
            .with_contract_context(ContractContextV1 {
                caller: caller.unwrap_or_default(),
                ..Default::default()
            })
            .with_devnet_genesis()
            .with_input(input)
    }

    #[serial]
    #[test]
    pub fn test_primitive_storage_dynamic_bytes() {
        let mut sdk = with_test_input(vec![], None);
        let b = fluentbase_sdk::Bytes::from(
            "this is a really long string. this is a really long
    string. this is a really long string. this it really long string.",
        );
        Data::set(&mut sdk, b.clone());
        let result = Data::get(&sdk);
        assert_eq!(result, b);
    }

    #[serial]
    #[test]
    pub fn test_nested_arr() {
        let mut sdk = with_test_input(vec![], None);
        let idx1 = U256::from(0);
        let idx2 = U256::from(0);
        let idx3 = U256::from(0);
        let value = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        NestedArr::set(&mut sdk, idx1, idx2, idx3, value);
        let result = NestedArr::get(&sdk, idx1, idx2, idx3);
        assert_eq!(result, value);
    }

    #[serial]
    #[test]
    pub fn test_storage_mapping() {
        let mut sdk = with_test_input(vec![], None);
        let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let value: U256 = U256::from_str_radix("1000000000000000000", 10).unwrap(); // 1000
        Balance::set(&mut sdk, addr, value);
        let result = Balance::get(&sdk, addr);
        assert_eq!(result, value);
    }

    #[serial]
    #[test]
    pub fn test_storage_mapping_nested() {
        let mut sdk = with_test_input(vec![], None);
        let addr1 = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let addr2 = Address::from(hex!("70997970C51812dc3A010C7d01b50e0d17dc79C8"));
        let value: U256 = U256::from_str_radix("1000000000000000000", 10).unwrap();
        Allowance::set(&mut sdk, addr1, addr2, value);
        let result = Allowance::get(&sdk, addr1, addr2);
        assert_eq!(result, result);
    }

    #[serial]
    #[test]
    pub fn test_storage_primitive_address() {
        let mut sdk = with_test_input(vec![], None);
        let addr1 = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        Owner::set(&mut sdk, addr1);
        let result = Owner::get(&sdk);
        assert_eq!(result, result);
    }

    #[serial]
    #[test]
    pub fn test_storage_primitive_u256() {
        let mut sdk = with_test_input(vec![], None);
        let a = U256::from(1);
        CustomU256::set(&mut sdk, a.clone());
        let result = CustomU256::get(&sdk);
        assert_eq!(result, a);
    }

    #[serial]
    #[test]
    pub fn test_storage_primitive_i256() {
        let mut sdk = with_test_input(vec![], None);
        let a = I256::try_from(I256::MIN).unwrap();
        CustomI256::set(&mut sdk, a.clone());
        let result = CustomI256::get(&sdk);
        assert_eq!(result, a);
    }

    #[serial]
    #[test]
    pub fn test_storage_primitive_struct() {
        let mut sdk = with_test_input(vec![], None);
        let a = U256::from(1);
        let b = U256::from(2);
        let c = fluentbase_sdk::Bytes::from(
            "this it really long string. this it really long
    string. this it really long string. this it really long string.",
        );
        let d = fluentbase_sdk::Bytes::from("short");
        let my_struct = MyStruct { a, b, c, d };
        SomeStruct::set(&mut sdk, my_struct.clone());
        let result = SomeStruct::get(&sdk);
        assert_eq!(result, my_struct);
    }

    #[serial]
    #[test]
    pub fn test_storage_mapping_struct() {
        let mut sdk = with_test_input(vec![], None);
        let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        let a = U256::from(1);
        let b = U256::from(2);
        let c = fluentbase_sdk::Bytes::from(
            "this it really long string. this it really long
    string. this it really long string. this it really long string.",
        );
        let d = fluentbase_sdk::Bytes::from("short");
        let my_struct = MyStruct { a, b, c, d };
        MyStructMap::set(&mut sdk, addr, my_struct.clone());
        let result = MyStructMap::get(&sdk, addr);
        assert_eq!(result, my_struct.clone());
    }

    #[serial]
    #[test]
    pub fn test_storage_key_reference_values() {
        let sdk = with_test_input(vec![], None);

        // Test slot values
        assert_eq!(CustomFixedBytes::SLOT, U256::from_limbs([0, 0, 0, 0]));
        assert_eq!(CustomFixedBytesArray::SLOT, U256::from_limbs([1, 0, 0, 0]));
        assert_eq!(Balance::SLOT, U256::from_limbs([6, 0, 0, 0]));
        assert_eq!(Allowance::SLOT, U256::from_limbs([7, 0, 0, 0]));
        assert_eq!(Owner::SLOT, U256::from_limbs([10, 0, 0, 0]));

        let addr1 = Address::from(hex!("f39fd6e51aad88f6f4ce6ab8827279cfffb92266"));
        let addr2 = Address::from(hex!("70997970c51812dc3a010c7d01b50e0d17dc79c8"));

        // Balance[addr1]
        // Expected format: keccak256(abi.encodePacked(bytes32(address), bytes32(slot)))
        let expected_balance_key = U256::from_str_radix(
            "c50c4d60f8bbb6a70920d195c8852bc6d816d9f7bc643b500261fc4d9a03f08c",
            16,
        )
        .unwrap();
        let actual_balance_key = Balance::key(&sdk, addr1);
        assert_eq!(
            actual_balance_key, expected_balance_key,
            "Balance key mismatch: expected {:?}, got {:?}",
            expected_balance_key, actual_balance_key
        );

        // Allowance[addr1][addr2]
        // Expected format: keccak256(abi.encodePacked(bytes32(addr2),
        // keccak256(abi.encodePacked(bytes32(addr1), bytes32(slot)))))
        let expected_allowance_key = U256::from_str_radix(
            "9497c69828ddf28f6ad649ddac9a7c28d7e9228a5a06a6acf21099fe94d38327",
            16,
        )
        .unwrap();
        let actual_allowance_key = Allowance::key(&sdk, addr1, addr2);
        assert_eq!(
            actual_allowance_key, expected_allowance_key,
            "Allowance key mismatch: expected {:?}, got {:?}",
            expected_allowance_key, actual_allowance_key
        );

        // Arr[42]
        // Expected format: keccak256(abi.encodePacked(bytes32(slot))) + index
        let idx = U256::from(42);
        let expected_array_key = U256::from_str_radix(
            "f3f7a9fe364faab93b216da50a3214154f22a0a2b415b23a84c8169e8b636f0d",
            16,
        )
        .unwrap();
        let actual_array_key = Arr::key(&sdk, idx);
        assert_eq!(
            actual_array_key, expected_array_key,
            "Array key mismatch: expected {:?}, got {:?}",
            expected_array_key, actual_array_key
        );

        // NestedArr[0][0][0]
        // Complex nested array calculation
        let idx1 = U256::from(0);
        let idx2 = U256::from(0);
        let idx3 = U256::from(0);
        let expected_nested_key = U256::from_str_radix(
            "7e36832397f38490551808dffc4af389da37450fee8ca1202b2419425bcdb132",
            16,
        )
        .unwrap();
        let actual_nested_key = NestedArr::key(&sdk, idx1, idx2, idx3);
        assert_eq!(
            actual_nested_key, expected_nested_key,
            "NestedArr key mismatch: expected {:?}, got {:?}",
            expected_nested_key, actual_nested_key
        );
    }
}
