use crate::{
    encoder::{
        align_up,
        is_big_endian,
        read_u32_aligned,
        write_u32_aligned,
        Encoder,
        FluentABI,
        SolidityABI,
    },
    error::CodecError,
    Codec,
};
use alloc::vec;
use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use alloy_sol_types::{
    sol,
    sol_data::{self},
    SolType,
    SolValue,
};
use byteorder::{ByteOrder, BE, LE};
use bytes::{Buf, BytesMut};
use hashbrown::HashMap;
use hex_literal::hex;

pub fn print_bytes<B: ByteOrder, const ALIGN: usize>(buf: &[u8]) {
    for (i, chunk) in buf.chunks(ALIGN).enumerate() {
        let offset = i * ALIGN;
        print!("{:04x}: ", offset);

        if is_big_endian::<B>() {
            for &byte in &chunk[&chunk.len() - 4..] {
                print!("{:02x} ", byte);
            }
        } else {
            for &byte in &chunk[..4] {
                print!("{:02x} ", byte);
            }
        }

        for _ in chunk.len()..ALIGN {
            print!("   ");
        }
        print!("  ||  {:03}", offset);
        let decimal_value = read_u32_aligned::<B, ALIGN>(&chunk, 0).unwrap();
        println!(": {:03} |", decimal_value);
    }
}

#[derive(Codec, Default, Debug, PartialEq)]
struct TestStruct {
    bool_val: bool,
    u8_val: u8,
    uint_val: (u16, u32, u64),
    int_val: (i16, i32, i64),
    u256_val: U256,
    address_val: Address,
    bytes_val: Bytes,
    vec_val: Vec<u32>,
}

#[test]
fn test_struct_1_field() {
    sol! {
        struct TestStructSol {
            bytes bytes_val;
        }
    }

    let test_struct = TestStructSol {
        bytes_val: Bytes::from("Hello, World!!".as_bytes()),
    };

    let sol_encoded = &test_struct.abi_encode();

    println!("{:?}", hex::encode(&sol_encoded));

    #[derive(Codec, Default, Debug, PartialEq)]
    struct TestStruct {
        bytes_val: Bytes,
    }
    let test_struct = TestStruct {
        bytes_val: Bytes::from("Hello, World!!".as_bytes()),
    };

    let mut buf = BytesMut::new();

    SolidityABI::encode(&test_struct, &mut buf, 0).unwrap();

    let fluent_encoded = buf.freeze();

    println!("{:?}", hex::encode(&fluent_encoded));

    // assert_eq!(sol_encoded, fluent_encoded);
}

#[test]
fn test_tuple_bytes_address_sol() {
    let b = Bytes::from("Hello, World!!".as_bytes());
    let a = Address::repeat_byte(0xAA);

    let tuple = (b.clone(), a.clone());

    type TupleType = sol!(tuple(bytes, address));

    let encoded_sol = tuple.abi_encode();

    // println!("Encoded data sol: 0x{}", hex::encode(&encoded_sol));
    print_bytes::<BE, 32>(&encoded_sol);
    let decoded_sol = TupleType::abi_decode(&encoded_sol, false).unwrap();

    assert_eq!(tuple, decoded_sol);

    type TupleTypeMy = (Bytes, Address);

    let mut buf = BytesMut::new();
    let tuple = (b.clone(), a.clone());
    SolidityABI::encode(&tuple, &mut buf, 0).unwrap();

    let encoded = buf.freeze();

    println!("Encoded data our: 0x{}", hex::encode(&encoded));

    assert_eq!(encoded.to_vec(), encoded_sol);

    let decoded = SolidityABI::<TupleTypeMy>::decode(&&encoded_sol[..], 0).unwrap();

    assert_eq!(tuple, decoded);
}

#[test]
fn test_tuple_single_sol() {
    let b = Bytes::from("Hello, World!!".as_bytes());

    type TupleType1 = sol!(tuple(bytes,));

    let tuple = (b.clone(),);
    let encoded_sol = tuple.abi_encode();

    let decoded_sol = TupleType1::abi_decode(&encoded_sol, false).unwrap();

    assert_eq!(tuple, decoded_sol);

    type TupleTypeMy = (Bytes,);

    let mut buf = BytesMut::new();
    let tuple = (b.clone(),);
    SolidityABI::encode(&tuple, &mut buf, 0).unwrap();

    let encoded = buf.freeze();

    assert_eq!(encoded.to_vec(), encoded_sol);

    let decoded = SolidityABI::<TupleTypeMy>::decode(&&encoded_sol[..], 0).unwrap();

    assert_eq!(tuple, decoded);
}

#[test]
fn test_struct_sol() {
    // Create an instance of TestStruct
    let test_struct = TestStruct {
        bool_val: true,
        u8_val: 42,
        uint_val: (1000, 1_000_000, 1_000_000_000),
        int_val: (-1000, -1_000_000, -1_000_000_000),
        u256_val: U256::from(12345),
        address_val: Address::repeat_byte(0xAA),
        bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
        vec_val: vec![10, 20, 30],
    };

    // Encode using our Encoder
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_struct, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    println!("{:?}", hex::encode(&encoded));

    print_bytes::<BE, 32>(&encoded);

    // Create an equivalent structure in alloy_sol_types
    sol! {
        struct TestStructSol {
            bool bool_val;
            uint8 u8_val;
            (uint16, uint32, uint64) uint_val;
            (int16, int32, int64) int_val;
            uint256 u256_val;
            address address_val;
            bytes bytes_val;
            uint32[] vec_val;
        }
    }

    let test_struct_sol = TestStructSol {
        bool_val: true,
        u8_val: 42,
        uint_val: (1000, 1_000_000, 1_000_000_000),
        int_val: (-1000, -1_000_000, -1_000_000_000),
        u256_val: U256::from(12345),
        address_val: Address::repeat_byte(0xAA),
        bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
        vec_val: vec![10, 20, 30],
    };

    let alloy_encoded = &test_struct_sol.abi_encode();
    println!("Alloy Encoded:");
    println!("{:?}", hex::encode(&alloy_encoded));
    print_bytes::<BE, 32>(&alloy_encoded);

    // Compare the results
    assert_eq!(
        hex::encode(&encoded),
        hex::encode(alloy_encoded),
        "Encoding mismatch between our Encoder and alloy_sol_types"
    );

    // Additionally, we can test decoding
    let decoded = SolidityABI::<TestStruct>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_struct, "Decoding mismatch");
}

#[derive(Codec, Default, Debug, PartialEq)]
struct TestStructSmall {
    bool_val: bool,
    bytes_val: Bytes,
    vec_val: Vec<u32>,
}

#[derive(Codec, Default, Debug, PartialEq)]
struct TestNestedStruct {
    nested_struct: TestStructSmall,
    fixed_bytes: [FixedBytes<32>; 2],
    uint_val: u32,
    vec_val: Vec<u32>,
}

#[test]
fn test_nested_struct_sol() {
    // Create equivalent structures in alloy_sol_types
    sol! {
        struct TestStructSmallSol {
            bool bool_val;
            bytes bytes_val;
            uint32[] vec_val;
        }

        struct TestNestedStructSol {
            TestStructSmallSol nested_struct;
            bytes32[2] fixed_bytes;
            uint32 uint_val;
            uint32[] vec_val;
        }
    }

    let test_struct_sol = TestStructSmallSol {
        bool_val: true,
        bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
        vec_val: vec![10, 20, 30],
    };

    let test_nested_struct_sol = TestNestedStructSol {
        nested_struct: test_struct_sol,
        fixed_bytes: [
            FixedBytes::<32>::from_slice(&[0x11; 32]),
            FixedBytes::<32>::from_slice(&[0x11; 32]),
        ],
        uint_val: 42,
        vec_val: vec![100, 200, 300],
    };

    let alloy_encoded = &test_nested_struct_sol.abi_encode();
    println!("Alloy Encoded:");
    println!("{:?}", hex::encode(&alloy_encoded));
    print_bytes::<BE, 32>(&alloy_encoded);

    // Create an instance of TestStruct
    let test_struct = TestStructSmall {
        bool_val: true,
        bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
        vec_val: vec![10, 20, 30],
    };

    // Create an instance of TestNestedStruct
    let test_nested_struct = TestNestedStruct {
        nested_struct: test_struct,
        fixed_bytes: [
            FixedBytes::<32>::from_slice(&[0x11; 32]),
            FixedBytes::<32>::from_slice(&[0x11; 32]),
        ],
        uint_val: 42,
        vec_val: vec![100, 200, 300],
    };

    // Encode using our Encoder
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_nested_struct, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    println!("Our Encoded:");
    println!("{:?}", hex::encode(&encoded));
    print_bytes::<BE, 32>(&encoded);

    // Compare the results
    assert_eq!(
        hex::encode(&encoded),
        hex::encode(alloy_encoded),
        "Encoding mismatch between our Encoder and alloy_sol_types"
    );

    println!("Decoding...");

    // Test decoding
    let decoded = SolidityABI::<TestNestedStruct>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_nested_struct, "Decoding mismatch");
}

#[derive(Codec, Default, Debug, PartialEq)]
struct TestStruct2 {
    bool_val: bool,
    bytes_val: Bytes,
    vec_val: Vec<u32>,
}

#[test]
fn test_struct_wasm() {
    let test_struct = TestStruct2 {
        bool_val: true,
        bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
        vec_val: vec![10, 20, 30],
    };

    let mut buf = BytesMut::new();
    FluentABI::encode(&test_struct, &mut buf, 0).unwrap();

    let encoded = buf.freeze();

    println!("encoded: {:?}", hex::encode(&encoded));

    let decoded = FluentABI::<TestStruct2>::decode(&encoded, 0).unwrap();

    assert_eq!(decoded, test_struct);
}

#[test]
fn test_fixed_sol() {
    let test_value: [u32; 3] = [0x11111111, 0x22222222, 0x33333333];

    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();

    let encoded = buf.freeze();

    let alloy_value = sol_data::FixedArray::<sol_data::Uint<32>, 3>::abi_encode(&test_value);

    println!("alloy encoded: {:?}", alloy_value);

    assert_eq!(encoded, alloy_value);

    let decoded = SolidityABI::<[u32; 3]>::decode(&&alloy_value[..], 0).unwrap();

    assert_eq!(decoded, test_value);
}

#[test]
fn test_bytes_sol() {
    let original = alloy_primitives::Bytes::from_static(b"hello world");

    let mut buf = BytesMut::new();
    SolidityABI::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();
    let expected = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 11, 104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    assert_eq!(encoded.to_vec(), expected);

    let sol_encoded = sol_data::Bytes::abi_encode(&original);

    assert_eq!(encoded.to_vec(), sol_encoded);

    let (offset, length) =
        SolidityABI::<alloy_primitives::Bytes>::partial_decode(&&sol_encoded[..], 0).unwrap();
    assert_eq!(offset, 32);
    assert_eq!(length, 11);

    let decoded = SolidityABI::<alloy_primitives::Bytes>::decode(&&sol_encoded[..], 0).unwrap();

    let alloy_decoded = sol_data::Bytes::abi_decode(&sol_encoded, false).unwrap();

    println!("Decoded Bytes (our): {:?}", decoded.to_vec());
    println!("Decoded Bytes (alloy): {:?}", alloy_decoded.to_vec());

    assert_eq!(decoded, original);
}

#[test]
fn test_tuple_wasm() {
    let b = Bytes::from("Hello, World!!".as_bytes());
    let a = Address::repeat_byte(0xAA);

    let t = (b, a);

    let mut buf = BytesMut::new();
    FluentABI::encode(&t, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    // println!("encoded: {:?}", hex::encode(&encoded));
    let expected_encoded = "040000001c0000000e000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa48656c6c6f2c20576f726c6421210000";

    assert_eq!(hex::encode(&encoded), expected_encoded);
    println!("Decoding...");
    let decoded = FluentABI::<(Bytes, Address)>::decode(&encoded, 0).unwrap();

    assert_eq!(decoded, t);
}
#[test]
fn test_fixed_bytes_sol() {
    // Use FixedBytes<11> to match the length of "hello world"
    let original = alloy_primitives::FixedBytes::<11>::from_slice(b"hello world");

    let mut buf = BytesMut::new();
    SolidityABI::<alloy_primitives::FixedBytes<11>>::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    // FixedBytes are encoded inline without length prefix
    let expected = [
        104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    assert_eq!(encoded.to_vec(), expected);

    // Encode using sol_data for comparison
    let sol_encoded = sol_data::FixedBytes::<11>::abi_encode(&original);

    assert_eq!(encoded.to_vec(), sol_encoded);

    // FixedBytes don't have a separate offset and length in their encoding
    let (offset, length) =
        SolidityABI::<alloy_primitives::FixedBytes<11>>::partial_decode(&&sol_encoded[..], 0)
            .unwrap();
    println!("Offset: {}, Length: {}", offset, length);
    assert_eq!(offset, 0); // FixedBytes are encoded inline
    assert_eq!(length, 32); // Always padded to 32 bytes

    let decoded =
        SolidityABI::<alloy_primitives::FixedBytes<11>>::decode(&&sol_encoded[..], 0).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_address_sol() {
    let original =
        alloy_primitives::Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

    let mut buf = BytesMut::new();
    SolidityABI::<alloy_primitives::Address>::encode(&original, &mut buf, 0).unwrap();

    let encoded = buf.freeze();

    let alloy_encoded = sol_data::Address::abi_encode(&original);

    assert_eq!(encoded.to_vec(), alloy_encoded);

    let decoded = SolidityABI::<alloy_primitives::Address>::decode(&&alloy_encoded[..], 0).unwrap();

    let alloy_decoded = sol_data::Address::abi_decode(&alloy_encoded, false).unwrap();

    assert_eq!(decoded, alloy_decoded);
    assert_eq!(decoded, original);
}

#[test]
fn test_vec_sol_simple() {
    let original: Vec<u32> = vec![1, 2, 3];
    let mut buf = BytesMut::new();
    SolidityABI::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    let alloy_value = sol_data::Array::<sol_data::Uint<32>>::abi_encode(&original);

    let expected_encoded = hex!(
        "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000003"
    );

    assert_eq!(encoded.to_vec(), expected_encoded);

    println!("Encoded Vec: {:?}", hex::encode(&encoded));

    assert_eq!(hex::encode(encoded), hex::encode(&alloy_value));

    let decoded = SolidityABI::<Vec<u32>>::decode(&&alloy_value[..], 0).unwrap();
    println!("Decoded Vec: {:?}", decoded);
    assert_eq!(decoded, original);
}

#[test]
fn test_vec_sol_nested() {
    let original: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];
    let mut buf = BytesMut::new();
    SolidityABI::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    let alloy_value = sol_data::Array::<sol_data::Array<sol_data::Uint<32>>>::abi_encode(&original);

    let expected_encoded = hex!(
        "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000c00000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000005"
    );
    println!("codec");
    print_bytes::<BE, 32>(&encoded);
    println!("solidity");
    print_bytes::<BE, 32>(&alloy_value);
    // println!("Encoded Vec: {:?}", hex::encode(&encoded));
    assert_eq!(encoded.to_vec(), alloy_value.to_vec());

    let decoded_alloy = sol_data::Array::<sol_data::Array<sol_data::Uint<32>>>::abi_decode(
        &expected_encoded,
        false,
    )
    .unwrap();
    println!("Decoded Vec: {:?}", decoded_alloy);

    assert_eq!(hex::encode(encoded), hex::encode(&alloy_value));

    let decoded = SolidityABI::<Vec<Vec<u32>>>::decode(&&alloy_value[..], 0).unwrap();
    println!("Decoded Vec: {:?}", decoded);
    assert_eq!(decoded, original);
}

#[test]
fn test_vec_wasm_nested() {
    let original: Vec<Vec<u32>> = vec![vec![1u32, 2, 3], vec![4, 5], vec![6, 7, 8, 9, 10]];
    let mut buf = BytesMut::new();
    FluentABI::<Vec<Vec<u32>>>::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    println!("Encoded Vec: {:?}", hex::encode(&encoded));

    let expected_encoded = hex!("030000000c0000004c00000003000000240000000c0000000200000030000000080000000500000038000000140000000100000002000000030000000400000005000000060000000700000008000000090000000a000000");

    assert_eq!(encoded.to_vec(), expected_encoded);

    let decoded = FluentABI::<Vec<Vec<u32>>>::decode(&&encoded[..], 0).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_vec_empty_sol() {
    let empty_vec: Vec<u32> = vec![];

    let mut buf = BytesMut::new();
    SolidityABI::<Vec<u32>>::encode(&empty_vec, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    let alloy_value = sol_data::Array::<sol_data::Uint<32>>::abi_encode(&empty_vec);
    println!("Encoded Vec: {:?}", hex::encode(&encoded));
    assert_eq!(encoded, alloy_value);

    let decoded = SolidityABI::<Vec<u32>>::decode(&&alloy_value[..], 0).unwrap();
    assert_eq!(decoded, empty_vec);
}
#[test]
fn test_bytes_empty_sol() {
    let original: alloy_primitives::Bytes = alloy_primitives::Bytes::new();
    let mut buf = BytesMut::new();

    SolidityABI::<alloy_primitives::Bytes>::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    println!("Encoded Bytes: {:?}", hex::encode(&encoded));

    let alloy_encoded = sol_data::Bytes::abi_encode(&original);

    assert_eq!(encoded, alloy_encoded);

    let decoded = SolidityABI::<alloy_primitives::Bytes>::decode(&&alloy_encoded[..], 0).unwrap();
    println!("Decoded Bytes: {:?}", decoded.to_vec());

    assert_eq!(original, decoded);
}

#[test]
fn test_vec_partial_decoding_sol() {
    let original: Vec<u32> = vec![1u32, 2, 3, 4, 5];
    let mut buf = BytesMut::new();
    SolidityABI::<Vec<u32>>::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    let alloy_value = sol_data::Array::<sol_data::Uint<32>>::abi_encode(&original);

    println!("Encoded Vec: {:?}", hex::encode(&encoded));

    assert_eq!(hex::encode(encoded), hex::encode(&alloy_value));

    // offset, length
    let decoded_header = SolidityABI::<Vec<u32>>::partial_decode(&&alloy_value[..], 0).unwrap();

    assert_eq!(decoded_header, (32, 5));
}

#[test]
fn test_vec_partial_decoding_wasm() {
    let original: Vec<u32> = vec![1u32, 2, 3, 4, 5];
    let mut buf = BytesMut::new();
    FluentABI::<Vec<u32>>::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    println!("Encoded Vec: {:?}", hex::encode(&encoded));

    // offset, length
    let decoded_header = FluentABI::<Vec<u32>>::partial_decode(&&encoded[..], 4).unwrap();
    assert_eq!(decoded_header, (12, 20));
    assert_eq!(encoded.chunk()[12..20], vec![1, 0, 0, 0, 2, 0, 0, 0]);
}

#[test]
fn test_map_sol_simple() {
    let mut original = HashMap::new();
    original.insert(10, 20);
    original.insert(1, 5);
    original.insert(100, 60);

    let mut buf = BytesMut::new();
    SolidityABI::encode(&original, &mut buf, 0).unwrap();

    let encoded = buf.freeze();
    println!("Encoded Map: {:?}", hex::encode(&encoded));

    let expected_encoded = hex!(
        "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000064000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000000000000000000000000003c"
    );

    assert_eq!(encoded.to_vec(), expected_encoded);

    print_bytes::<BE, 32>(&encoded.chunk());

    let decoded = SolidityABI::<HashMap<u32, u32>>::decode(&&encoded[..], 0).unwrap();

    assert_eq!(decoded, original);
}
#[test]
fn test_map_sol_nested() {
    let mut original = HashMap::new();
    original.insert(9, HashMap::from([(8, 7)]));

    println!("Original Map: {:?}", original);

    let mut buf = BytesMut::new();
    SolidityABI::encode(&original, &mut buf, 0).unwrap();

    let encoded = buf.freeze();
    println!("Encoded Map: {:?}", hex::encode(&encoded));

    print_bytes::<BE, 32>(&encoded.chunk());

    // let expected_encoded =
    // "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000000000000000000000000001800000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000001c000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000005000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000008"
    // ;

    let decoded = SolidityABI::<HashMap<u32, HashMap<u32, u32>>>::decode(&&encoded[..], 0).unwrap();

    assert_eq!(decoded, original);
}
#[test]
fn test_map_sol_nested_2() {
    let mut original = HashMap::new();
    original.insert(1, HashMap::from([(5, 6)]));
    original.insert(2, HashMap::from([(7, 8)]));

    let mut buf = BytesMut::new();
    SolidityABI::encode(&original, &mut buf, 0).unwrap();

    let encoded = buf.freeze();

    print_bytes::<BE, 32>(&encoded.chunk());

    // println!("Encoded Map: {:?}", hex::encode(&encoded));

    // let expected_encoded =
    // "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000000000000000000000000001800000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000001c000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000005000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000008"
    // ;

    let decoded = SolidityABI::<HashMap<u32, HashMap<u32, u32>>>::decode(&&encoded[..], 0).unwrap();
    println!("Decoded Map: {:?}", decoded);

    assert_eq!(decoded, original);
}

#[test]
fn test_map_wasm_simple() {
    let mut original = HashMap::new();
    original.insert(100, 20);
    original.insert(3, 5);
    original.insert(1000, 60);

    let mut buf = BytesMut::new();
    FluentABI::encode(&original, &mut buf, 0).unwrap();

    let encoded = buf.freeze();

    let expected_encoded = hex!(
        "03000000140000000c000000200000000c0000000300000064000000e803000005000000140000003c000000"
    );

    assert_eq!(encoded.to_vec(), expected_encoded);

    println!("Encoded Map: {:?}", hex::encode(&encoded));

    let decoded = FluentABI::<HashMap<u32, u32>>::decode(&&encoded[..], 0).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_map_wasm_nested() {
    let mut original = HashMap::new();
    original.insert(1, HashMap::from([(5, 6)]));
    original.insert(2, HashMap::from([(7, 8)]));

    let mut buf = BytesMut::new();
    FluentABI::encode(&original, &mut buf, 0).unwrap();

    let encoded = buf.freeze();
    let expected_encoded =
    "0200000014000000080000001c0000003800000001000000020000000100000028000000040000002c00000004000000010000003000000004000000340000000400000005000000060000000700000008000000";

    assert_eq!(hex::encode(&encoded), expected_encoded, "Encoding mismatch");

    print_bytes::<LE, 4>(&encoded.chunk());

    let decoded = FluentABI::<HashMap<u32, HashMap<u32, u32>>>::decode(&&encoded[..], 0).unwrap();

    assert_eq!(decoded, original);
}
