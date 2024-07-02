#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use crate::storage::StorageValue;
use alloy_sol_types::SolValue;
use core::{borrow::Borrow, fmt::Debug};
use fluentbase_sdk::{
    codec::{BufferDecoder, Encoder, WritableBuffer},
    contracts::{EvmAPI, EvmClient, EvmSloadInput, EvmSstoreInput, PRECOMPILE_EVM},
    derive::solidity_storage,
    Address,
    Bytes,
    LowLevelSDK,
    SharedAPI,
    U256,
};

mod storage;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MyStruct {
    pub a: U256,
    pub b: U256,
}

impl Encoder<MyStruct> for MyStruct {
    const HEADER_SIZE: usize = U256::HEADER_SIZE + U256::HEADER_SIZE;

    fn encode<W: WritableBuffer>(&self, encoder: &mut W, field_offset: usize) {
        self.a.encode(encoder, field_offset);
        self.b.encode(encoder, field_offset + U256::HEADER_SIZE);
    }

    fn decode_header(
        decoder: &mut BufferDecoder,
        field_offset: usize,
        result: &mut MyStruct,
    ) -> (usize, usize) {
        let (a_size, a_offset) = U256::decode_header(decoder, field_offset, &mut result.a);
        let (b_size, b_offset) =
            U256::decode_header(decoder, field_offset + a_offset, &mut result.b);
        (a_size + b_size, a_offset + b_offset)
    }

    fn decode_body(decoder: &mut BufferDecoder, field_offset: usize, result: &mut MyStruct) {
        let (_, _) = U256::decode_header(decoder, field_offset, &mut result.a);
        U256::decode_body(decoder, field_offset, &mut result.a);

        U256::decode_body(decoder, 32, &mut result.b);
    }
}

impl<Client: EvmAPI> StorageValue<Client, MyStruct> for MyStruct {
    fn get(&self, client: &Client, key: U256) -> Result<MyStruct, String> {
        let chunk_size = 32;
        let num_chunks = 2;

        let mut buffer = Vec::with_capacity(num_chunks * chunk_size);
        for i in 0..num_chunks {
            let input = EvmSloadInput {
                index: key + U256::from(i),
            };
            let output = client.sload(input);
            let chunk = output.value.to_be_bytes::<32>();
            buffer.extend_from_slice(&chunk);
        }

        let mut decoder = BufferDecoder::new(&buffer);
        let mut body = MyStruct::default();
        MyStruct::decode_body(&mut decoder, 0, &mut body);
        Ok(body)
    }

    fn set(&self, client: &Client, key: U256, value: MyStruct) -> Result<(), String> {
        let encoded_buffer = value.encode_to_vec(0);
        let chunk_size = 32;
        let num_chunks = (encoded_buffer.len() + chunk_size - 1) / chunk_size;

        for i in 0..num_chunks {
            let start = i * chunk_size;
            let end = (start + chunk_size).min(encoded_buffer.len());
            let chunk = &encoded_buffer[start..end];

            let mut chunk_padded = [0u8; 32];
            chunk_padded[..chunk.len()].copy_from_slice(chunk);

            let value_u256 = U256::from_be_bytes(chunk_padded);
            let input = EvmSstoreInput {
                index: key + U256::from(i),
                value: value_u256,
            };

            client.sstore(input);
        }

        Ok(())
    }
}

solidity_storage! {
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
    U256[] Arr;
    Address[][][] NestedArr;
    Address Owner;
    Bytes Data;
    MyStruct SomeStruct;
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

    fn get_output() -> Vec<u8> {
        LowLevelSDK::get_test_output()
    }
    #[serial]
    #[test]
    pub fn test_primitive_storage_dynamic_bytes() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let b = fluentbase_sdk::Bytes::from(
            "this it really long string. this it really long
    string. this it really long string. this it really long string.",
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

        let my_struct = MyStruct { a, b };

        storage.set(my_struct.clone());

        let result = storage.get();

        assert_eq!(result, my_struct);
    }
}
