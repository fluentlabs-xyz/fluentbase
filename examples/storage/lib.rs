#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

use alloy_sol_types::{sol, SolValue};
use core::{borrow::Borrow, fmt::Debug};
use fluentbase_sdk::{
    bytes::buf,
    codec::{BufferDecoder, Encoder, WritableBuffer},
    contracts::{EvmAPI, EvmClient, EvmSloadInput, EvmSstoreInput, PRECOMPILE_EVM},
    derive::solidity_storage,
    Address,
    LowLevelSDK,
    SharedAPI,
    U256,
};
use hex_literal::hex;

solidity_storage! {
    U256[] Arr;
    // U256 Counter;
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
}
// mapping(Address => Book) BookStorage;

pub struct Counter<'a, T: EvmAPI + 'a> {
    client: &'a T,
}
impl<'a, T: EvmAPI + 'a> Counter<'a, T> {
    const SLOT: fluentbase_sdk::U256 = U256::from_limbs([0u64, 0u64, 0u64, 0u64]);
    pub fn new(client: &'a T) -> Self {
        Self { client }
    }
    pub fn key(&self) -> fluentbase_sdk::U256 {
        Self::SLOT
    }

    pub fn get<V: Encoder<V> + Default + Debug>(&self) -> Result<V, String> {
        if V::HEADER_SIZE == 32
            || V::HEADER_SIZE == 20
            || V::HEADER_SIZE == 16
            // || V::HEADER_SIZE == 8 // TODO: d1r1 we need to create more generic way to derrive is it dynamic or static. Now we are using it like a hack
            || V::HEADER_SIZE == 4
            || V::HEADER_SIZE == 2
            || V::HEADER_SIZE == 1
        {
            self.get_static::<V>()
        } else {
            self.get_dynamic::<V>()
        }
    }

    pub fn get_static<V: Encoder<V> + Default + Debug>(&self) -> Result<V, String> {
        let key = self.key();
        let input = EvmSloadInput { index: key };
        let output = self.client.sload(input);
        let chunk = output.value.to_be_bytes::<32>();
        let size = match V::HEADER_SIZE {
            32 => 0,  // uint256, int256, fixed256, ufixed256
            20 => 12, // address
            16 => 16, // uint128, int128, fixed128, ufixed128
            8 => 24,  // uint64, int64, fixed64, ufixed64
            4 => 28,  // uint32, int32, fixed32, ufixed32
            2 => 30,  // uint16, int16, fixed16, ufixed16
            1 => 31,  // uint8, int8, fixed8, ufixed8
            _ => return Err("Unsupported static type".to_string()),
        };

        let chunk = &chunk[..32 - size];

        let mut decoder = BufferDecoder::new(&chunk);
        let mut body = V::default();
        V::decode_body(&mut decoder, 0, &mut body);

        Ok(body)
    }

    pub fn get_dynamic<V: Encoder<V> + Default + Debug>(&self) -> Result<V, String> {
        let key = self.key();

        // Load the header
        let output = self.client.sload(EvmSloadInput { index: key });
        let header_chunk = output.value.to_be_bytes::<32>();

        let mut decoder = BufferDecoder::new(&header_chunk);

        // Decode the header to get offset and length of the data
        let (header_offset, data_len) = V::decode_header(&mut decoder, 0, &mut V::default());

        // Calculate the number of chunks to load
        let chunk_size = 32;
        let num_chunks = (data_len + chunk_size - 1) / chunk_size;

        let mut buffer = Vec::with_capacity(num_chunks * chunk_size);

        // Load all chunks of data
        for i in 0..num_chunks {
            let input = EvmSloadInput {
                index: key + U256::from(i + (header_offset / chunk_size)),
            };
            let output = self.client.sload(input);
            let chunk = output.value.to_be_bytes::<32>();

            buffer.extend_from_slice(&chunk);
        }

        // Trim the buffer to the actual length of the data
        buffer.truncate(header_offset + data_len);

        let mut decoder = BufferDecoder::new(&buffer);
        let mut body = V::default();
        V::decode_body(&mut decoder, 0, &mut body);

        Ok(body)
    }

    pub fn set<V: Encoder<V> + Debug>(&self, value: V) -> Result<(), String> {
        let key = self.key();
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
            self.client.sstore(input);
        }

        Ok(())
    }
}

impl<'a> Default for Counter<'a, EvmClient> {
    fn default() -> Self {
        Self {
            client: &EvmClient {
                address: PRECOMPILE_EVM,
                fuel: u32::MAX,
            },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use core::ops::Add;
    use fluentbase_sdk::{
        codec::Encoder,
        contracts::EvmClient,
        Address,
        Bytes,
        ContractInput,
        LowLevelSDK,
        U256,
    };
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
    // current test
    #[serial]
    #[test]
    pub fn test_primitive_storage_addr() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = Counter::default();
        let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

        storage.set(addr).unwrap();

        let result: Address = storage.get().unwrap();
        assert_eq!(result, addr);
    }
    #[serial]
    #[test]
    pub fn test_primitive_storage_u256() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = Counter::default();
        let num = U256::from_str_radix("1000000000000000000", 10).unwrap(); // 1000
        storage.set(num).unwrap();

        let result: U256 = storage.get().unwrap();
        assert_eq!(result, num);
    }

    #[serial]
    #[test]
    pub fn test_primitive_storage_bool() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let storage = Counter::default();
        let b = true;
        storage.set(b).unwrap();

        let result: bool = storage.get().unwrap();
        assert_eq!(result, b);
    }

    #[serial]
    #[test]
    pub fn test_primitive_storage_dynamic_bytes() {
        LowLevelSDK::init_with_devnet_genesis();
        with_test_input(vec![], None);

        let b = fluentbase_sdk::Bytes::from("this it really long string. this it really long string. this it really long string. this it really long string.");
        let storage = Counter::default();
        storage.set(b.clone()).unwrap();

        let result: fluentbase_sdk::Bytes = storage.get().unwrap();
        assert_eq!(result, b);
    }
}

// struct FieldStorage<V> {
//     _pd: PhantomData<V>,
// }
// struct MappingStorage<K, V> {
//     _pd0: PhantomData<K>,
//     _pd1: PhantomData<V>,
// }
// struct ArrayStorage<V> {
//     _pd: PhantomData<V>,
// }

// trait IMappingStorage {
//     fn storage_key(slot: U256, key: U256) -> U256;
// }

// impl<V> FieldStorage<V> {
//     pub fn storage_key(slot: U256) -> U256 {
//         slot
//     }
// }
// impl<K, V> MappingStorage<K, V> {
//     pub fn storage_key(slot: U256, key: U256) -> U256 {
//         let mut raw_storage_key: [u8; 64] = [0; 64];
//         raw_storage_key[0..32].copy_from_slice(slot.as_le_slice());
//         raw_storage_key[32..64].copy_from_slice(key.as_le_slice());
//         let mut storage_key: [u8; 32] = [0; 32];
//         LowLevelSDK::keccak256(
//             raw_storage_key.as_ptr(),
//             raw_storage_key.len() as u32,
//             storage_key.as_mut_ptr(),
//         );
//         U256::from_be_bytes(storage_key)
//     }
// }
// impl<V> ArrayStorage<V> {
//     pub fn storage_key(slot: U256, index: U256) -> U256 {
//         let mut storage_key: [u8; 32] = [0; 32];
//         LowLevelSDK::keccak256(slot.as_le_slice().as_ptr(), 32, storage_key.as_mut_ptr());
//         let storage_key = U256::from_be_bytes(storage_key);
//         storage_key + index
//     }
// }

// #[cfg(test)]
// mod test {
//     use super::*;
//     use core::ops::Add;
//     use fluentbase_sdk::{
//         codec::Encoder,
//         contracts::EvmClient,
//         Address,
//         Bytes,
//         ContractInput,
//         LowLevelSDK,
//         U256,
//     };
//     use serial_test::serial;

//     fn with_test_input<T: Into<Bytes>>(input: T, caller: Option<Address>) {
//         let mut contract_input = ContractInput::default();
//         contract_input.contract_caller = caller.unwrap_or_default();

//         LowLevelSDK::with_test_context(contract_input.encode_to_vec(0));
//         let input: Bytes = input.into();
//         LowLevelSDK::with_test_input(input.into());
//     }

//     fn get_output() -> Vec<u8> {
//         LowLevelSDK::get_test_output()
//     }
//     // #[test]
//     // pub fn test_mapping_with_struct_value() {
//     //     let storage = BookStorage::default();

//     //     let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

//     //     LowLevelSDK::init_with_devnet_genesis();
//     //     with_test_input(vec![], Some(addr));

//     //     let book = Book {
//     //         title: "The Great Gatsby".to_string(),
//     //         author: "F. Scott Fitzgerald".to_string(),
//     //         book_id: U256::from(1),
//     //     };

//     //     let book_bytes = book.abi_encode_packed();

//     //     storage.set(addr, book);

//     //     let result = storage.get(addr);
//     //     assert_eq!(result, book);
//     // }

//     #[test]
//     pub fn test_mapping_storage() {
//         let storage = Balance::default();
//         let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

//         LowLevelSDK::init_with_devnet_genesis();
//         with_test_input(vec![], Some(addr));

//         let value: U256 = U256::from_str_radix("1000000000000000000", 10).unwrap(); // 1000
//         storage.set(addr, value);

//         let result = storage.get(addr);
//         assert_eq!(result, value);
//     }

//     #[serial]
//     #[test]
//     pub fn test_nested_mapping_storage() {
//         let storage = Allowance::default();
//         let addr1 = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
//         let addr2 = Address::from(hex!("70997970C51812dc3A010C7d01b50e0d17dc79C8"));

//         LowLevelSDK::init_with_devnet_genesis();
//         with_test_input(vec![], Some(addr1));

//         let value: U256 = U256::from_str_radix("1000000000000000000", 10).unwrap(); // 1000

//         storage.set(addr1, addr2, value);

//         let result = storage.get(addr1, addr2);
//         assert_eq!(result, value);
//     }
//     // current test
//     #[test]
//     pub fn test_primitive_storage() {
//         let storage = Counter::default();
//         let addr = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

//         LowLevelSDK::init_with_devnet_genesis();
//         with_test_input(vec![], None);

//         storage.set(addr);

//         let result: Address = storage.get();
//         assert_eq!(result, addr);

//         let num = U256::from_str_radix("1000000000000000000", 10).unwrap(); // 1000
//         let storage = Counter::default();
//         storage.set(num);

//         let result: U256 = storage.get();
//         assert_eq!(result, num);
//     }
//     #[test]
//     pub fn test_primitive_storage_dynamic_bytes() {
//         LowLevelSDK::init_with_devnet_genesis();
//         with_test_input(vec![], None);

//         let b = fluentbase_sdk::Bytes::from("hello world");
//         let storage = Counter::default();
//         storage.set(b.clone());

//         let result: fluentbase_sdk::Bytes = storage.get();
//         assert_eq!(result, b);
//     }
//     #[test]
//     pub fn test_primitive_storage_dynamic_nums() {
//         LowLevelSDK::init_with_devnet_genesis();
//         with_test_input(vec![], None);

//         let n = U256::from_str_radix("1000000000000000000", 10).unwrap(); // 1000 * 10**18
//         let storage = Counter::default();
//         storage.set(n);

//         let result: fluentbase_sdk::U256 = storage.get();
//         assert_eq!(result, n);
//     }

//     // #[serial]
//     // #[test]
//     // pub fn test_arr() {
//     //     let client = EvmClient::new(PRECOMPILE_EVM);
//     //     let arr = Arr::new(&client);
//     //     let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
//     //     LowLevelSDK::init_with_devnet_genesis();

//     //     with_test_input(vec![], Some(owner_address));
//     //     let owner_balance: U256 = U256::from_str_radix("1000000000000000000000", 10).unwrap();
// //     // 1000

//     //     let index = U256::from_str_radix("0", 10).unwrap();

//     //     arr.set(index, owner_balance);

//     //     let output = arr.get(index);

//     //     assert_eq!(output, owner_balance);
//     // }

//     #[serial]
//     #[test]
//     pub fn test_storage() {
//         let owner_address = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
//         LowLevelSDK::init_with_devnet_genesis();
//         with_test_input(vec![], Some(owner_address));
//         let owner_balance: U256 = U256::from_str_radix("1000000000000000000000", 10).unwrap(); //
// 1000

//         let slot = U256::from_str_radix("1", 10).unwrap();
//         let input = EvmSstoreInput {
//             index: slot,
//             value: owner_balance,
//         };

//         let client = EvmClient::new(PRECOMPILE_EVM);
//         client.sstore(input);

//         let sload_input = EvmSloadInput { index: slot };

//         let balance = client.sload(sload_input);

//         assert_eq!(balance.value, owner_balance);
//     }
// }
