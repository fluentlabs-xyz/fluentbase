use crate::{
    encoder::{is_big_endian, read_u32_aligned, Encoder, FluentABI, SolidityABI},
    test_utils::print_bytes,
    Codec,
};
use alloc::vec;
use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use byteorder::{ByteOrder, BE, LE};
use bytes::{Buf, BytesMut};
use hashbrown::HashMap;
use hex_literal::hex;

#[test]
fn test_simple_struct_solidity() {
    let expected_encoded = hex::decode(
        "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000"
    ).unwrap();

    #[derive(Codec, Default, Debug, PartialEq)]
    struct TestStruct {
        bytes_val: Bytes,
    }

    let test_struct = TestStruct {
        bytes_val: Bytes::from("Hello, World!!".as_bytes()),
    };

    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_struct, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(expected_encoded, encoded.to_vec());

    let decoded = SolidityABI::<TestStruct>::decode(&&encoded[..], 0).unwrap();

    assert_eq!(decoded, test_struct);
}

#[test]
fn test_complex_struct_solidity() {
    #[derive(Debug, PartialEq, Codec)]
    struct TestStructFluent {
        u_8: u8,
        u_16: u16,
        u_32: u32,
        u_64: u64,
        boolean: bool,
        bytes_1: [u8; 1],
        bytes_32: [u8; 32],
        bytes: Vec<u8>,
        dynamic_array: Vec<u64>,
    }

    let original = TestStructFluent {
        u_8: 255,
        u_16: 65535,
        u_32: 4294967295,
        u_64: 18446744073709551615,
        boolean: true,
        bytes_1: [0xFF],
        bytes_32: [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ],
        bytes: ::alloc::vec![
            0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD,
            0xFE, 0xFF
        ],
        dynamic_array: ::alloc::vec![
            18446744073709551615,
            9223372036854775807,
            4611686018427387903,
            1,
        ],
    };

    let expected_encoded = "000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000ff000000000000000000000000000000000000000000000000000000000000ffff00000000000000000000000000000000000000000000000000000000ffffffff000000000000000000000000000000000000000000000000ffffffffffffffff000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000ff000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000700000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000009000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000b000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000d000000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000000f0000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000120000000000000000000000000000000000000000000000000000000000000013000000000000000000000000000000000000000000000000000000000000001400000000000000000000000000000000000000000000000000000000000000150000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000001700000000000000000000000000000000000000000000000000000000000000180000000000000000000000000000000000000000000000000000000000000019000000000000000000000000000000000000000000000000000000000000001a000000000000000000000000000000000000000000000000000000000000001b000000000000000000000000000000000000000000000000000000000000001c000000000000000000000000000000000000000000000000000000000000001d000000000000000000000000000000000000000000000000000000000000001e000000000000000000000000000000000000000000000000000000000000001f000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000005000000000000000000000000000000000000000000000000000000000000000720000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000f000000000000000000000000000000000000000000000000000000000000000f100000000000000000000000000000000000000000000000000000000000000f200000000000000000000000000000000000000000000000000000000000000f300000000000000000000000000000000000000000000000000000000000000f400000000000000000000000000000000000000000000000000000000000000f500000000000000000000000000000000000000000000000000000000000000f600000000000000000000000000000000000000000000000000000000000000f700000000000000000000000000000000000000000000000000000000000000f800000000000000000000000000000000000000000000000000000000000000f900000000000000000000000000000000000000000000000000000000000000fa00000000000000000000000000000000000000000000000000000000000000fb00000000000000000000000000000000000000000000000000000000000000fc00000000000000000000000000000000000000000000000000000000000000fd00000000000000000000000000000000000000000000000000000000000000fe00000000000000000000000000000000000000000000000000000000000000ff0000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000ffffffffffffffff0000000000000000000000000000000000000000000000007fffffffffffffff0000000000000000000000000000000000000000000000003fffffffffffffff0000000000000000000000000000000000000000000000000000000000000001";

    let mut buf = BytesMut::new();

    SolidityABI::encode(&original, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(expected_encoded, hex::encode(&encoded));

    let decoded = SolidityABI::<TestStructFluent>::decode(&encoded, 0).unwrap();

    assert_eq!(decoded, original);
}

#[test]
fn test_tuple_bytes_address_solidity() {
    // Expected encoding for tuple(bytes, address):
    // - First 32 bytes: offset to tuple data (32)
    // - Next 32 bytes: offset to bytes data (64)
    // - Next 32 bytes: address (padded to 32 bytes)
    // - Next 32 bytes: length of bytes (14)
    // - Final 32 bytes: bytes data (padded)
    let expected_encoded = hex::decode(concat!(
        // Tuple header
        "0000000000000000000000000000000000000000000000000000000000000020", // offset to tuple data
        // Tuple data
        "0000000000000000000000000000000000000000000000000000000000000040", // offset to bytes
        "000000000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", // address
        "000000000000000000000000000000000000000000000000000000000000000e", // bytes length (14)
        "48656c6c6f2c20576f726c642121000000000000000000000000000000000000"  /* "Hello, World!!"
                                                                             * padded */
    ))
    .unwrap();

    // Create test data
    let test_value = (Bytes::from("Hello, World!!"), Address::repeat_byte(0xAA));

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Tuple encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<(Bytes, Address)>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_tuple_single_solidity() {
    // Define expected encoding for tuple(bytes):
    // - First 32 bytes: offset to tuple data (32)
    // - Next 32 bytes: offset to bytes data (32)
    // - Next 32 bytes: length of bytes (14)
    // - Final 32 bytes: bytes data (padded)
    let expected_encoded = hex::decode(concat!(
        // Tuple header
        "0000000000000000000000000000000000000000000000000000000000000020", // offset to tuple data
        // Bytes header
        "0000000000000000000000000000000000000000000000000000000000000020", // offset to bytes data
        // Bytes data
        "000000000000000000000000000000000000000000000000000000000000000e", // length (14)
        "48656c6c6f2c20576f726c642121000000000000000000000000000000000000"  /* "Hello, World!!"
                                                                             * padded */
    ))
    .unwrap();

    // Create test data
    let test_value = (Bytes::from("Hello, World!!"),);

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Single-element tuple encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<(Bytes,)>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_struct_dynamic_solidity() {
    // Reference encoded data from Solidity ABI encoding
    let expected_encoded = hex::decode(
        "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002a00000000000000000000000000000000000000000000000000000000000003e800000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000003b9aca00fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc18fffffffffffffffffffffffffffffffffffffffffffffffffffffffffff0bdc0ffffffffffffffffffffffffffffffffffffffffffffffffffffffffc46536000000000000000000000000000000000000000000000000000000000000003039000000000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000001c0000000000000000000000000000000000000000000000000000000000000000501020304050000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000000000000000000000000001e"
    ).unwrap();

    // Test structure definition with various Solidity types
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

    // Create test structure instance with boundary values
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

    // Perform encoding using SolidityABI
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_struct, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    // Verify encoding matches Solidity reference
    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Encoding mismatch with expected data"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<TestStruct>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_struct, "Round-trip encoding/decoding failed");
}

#[test]
fn test_nested_struct_solidity() {
    // Test structures with different Solidity types
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

    // Reference encoded data from Solidity ABI encoding
    let expected_encoded = hex::decode(
        "000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000a011111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111000000000000000000000000000000000000000000000000000000000000002a00000000000000000000000000000000000000000000000000000000000001c00000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000000501020304050000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000000000000000000000000001e0000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000006400000000000000000000000000000000000000000000000000000000000000c8000000000000000000000000000000000000000000000000000000000000012c"
    ).unwrap();

    // Create inner struct instance
    let test_struct = TestStructSmall {
        bool_val: true,
        bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
        vec_val: vec![10, 20, 30],
    };

    // Create outer struct with nested data
    let test_nested_struct = TestNestedStruct {
        nested_struct: test_struct,
        fixed_bytes: [
            FixedBytes::<32>::from_slice(&[0x11; 32]),
            FixedBytes::<32>::from_slice(&[0x11; 32]),
        ],
        uint_val: 42,
        vec_val: vec![100, 200, 300],
    };

    // Encode struct using SolidityABI
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_nested_struct, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    // Verify encoding matches Solidity reference
    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Encoding mismatch with expected data"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<TestNestedStruct>::decode(&encoded, 0).unwrap();
    assert_eq!(
        decoded, test_nested_struct,
        "Round-trip encoding/decoding failed"
    );
}

// TODO: d1r1 Investigate dynamic offset requirements for FluentABI encoding
// Questions to address:
// - Should dynamic types (Bytes, Vec) have offset table like in SolidityABI?
// - How does FluentABI handle nested dynamic fields?
// - Do we need to modify the encoding format for better WASM compatibility?
//
// Related references:
// - Current test data layout: [bool][bytes_len][bytes_data][vec_len][vec_data]
// - Compare with SolidityABI dynamic encoding pattern
//
// Priority: HIGH
// Created: 2024-10-31
#[test]
fn test_struct_wasm() {
    #[derive(Codec, Default, Debug, PartialEq)]
    struct TestStruct {
        bool_val: bool,
        bytes_val: Bytes,
        vec_val: Vec<u32>,
    }
    // Reference encoded data for FluentABI format
    let expected_encoded = hex::decode(concat!(
        "01000000",                 // bool_val: true
        "18000000050000000",        // bytes_val offset and data size
        "3000000200000000c000000",  // vec_val length, offset and data length
        "0102030405000000",         // bytes_val data
        "0a000000140000001e000000", // vec_val data: [10, 20, 30]
    ))
    .unwrap();

    // Create test structure with sample data
    let test_struct = TestStruct {
        bool_val: true,
        bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
        vec_val: vec![10, 20, 30],
    };

    // Encode using FluentABI
    let mut buf = BytesMut::new();
    FluentABI::encode(&test_struct, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    // Verify encoding matches reference data
    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Encoding mismatch with expected data"
    );

    // Verify round-trip encoding/decoding
    let decoded = FluentABI::<TestStruct>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_struct, "Round-trip encoding/decoding failed");
}

#[test]
fn test_fixed_array_solidity() {
    // Test encoding/decoding of fixed-size array with known values
    let test_value: [u32; 3] = [0x11111111, 0x22222222, 0x33333333];
    let expected_encoded = concat!(
        "0000000000000000000000000000000000000000000000000000000011111111",
        "0000000000000000000000000000000000000000000000000000000022222222",
        "0000000000000000000000000000000000000000000000000000000033333333",
    );

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();
    assert_eq!(
        hex::encode(&encoded),
        expected_encoded,
        "Fixed array encoding mismatch"
    );

    // Verify round-trip
    let decoded = SolidityABI::<[u32; 3]>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Fixed array round-trip failed");
}

#[test]
fn test_bytes_solidity() {
    // Define expected encoding with clear structure:
    // - First 32 bytes: offset (32)
    // - Next 32 bytes: length (11)
    // - Final 32 bytes: data ("hello world" padded)
    let expected_encoded = hex::decode(concat!(
        "0000000000000000000000000000000000000000000000000000000000000020", // offset
        "000000000000000000000000000000000000000000000000000000000000000b", // length (11)
        "68656c6c6f20776f726c64000000000000000000000000000000000000000000"  // "hello world" padded
    ))
    .unwrap();

    // Create test bytes
    let test_value = Bytes::from_static(b"hello world");

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Bytes encoding doesn't match expected value"
    );

    // Verify correct offset and length
    let (offset, length) = SolidityABI::<Bytes>::partial_decode(&encoded, 0).unwrap();
    assert_eq!(offset, 32, "Incorrect dynamic data offset");
    assert_eq!(length, 11, "Incorrect data length");

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<Bytes>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_tuple_wasm() {
    // Define expected encoding with clear structure:
    // - First 4 bytes: offset to bytes data (4)
    // - Next 4 bytes: offset to address (28)
    // - Next 4 bytes: bytes length (14)
    // - Next 20 bytes: address data
    // - Final bytes: actual bytes data with padding
    let expected_encoded = hex::decode(concat!(
        "04000000",                                 // offset to bytes data
        "1c000000",                                 // offset to address (28)
        "0e000000",                                 // bytes length (14)
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", // address
        "48656c6c6f2c20576f726c642121",             // "Hello, World!!"
        "0000"                                      // padding
    ))
    .unwrap();

    // Create test data
    let test_value = (Bytes::from("Hello, World!!"), Address::repeat_byte(0xAA));

    // Encode and verify
    let mut buf = BytesMut::new();
    FluentABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Tuple encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = FluentABI::<(Bytes, Address)>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}
#[test]
fn test_fixed_bytes_solidity() {
    // Define expected encoding:
    // FixedBytes are encoded inline and right-padded with zeros to 32 bytes
    let expected_encoded = hex::decode(concat!(
        "68656c6c6f20776f726c64",                     // "hello world" (11 bytes)
        "000000000000000000000000000000000000000000"  // padding (21 bytes)
    ))
    .unwrap();

    // Create test fixed bytes
    let test_value = FixedBytes::<11>::from_slice(b"hello world");

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "FixedBytes encoding doesn't match expected value"
    );

    // Verify metadata for fixed-size types
    let (offset, length) = SolidityABI::<FixedBytes<11>>::partial_decode(&encoded, 0).unwrap();
    assert_eq!(
        offset, 0,
        "Fixed-size types should be encoded inline (offset = 0)"
    );
    assert_eq!(
        length, 32,
        "Fixed-size types should always occupy full 32-byte words"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<FixedBytes<11>>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_address_solidity() {
    // Define expected encoding:
    // Addresses are left-padded with zeros to 32 bytes (12 bytes padding + 20 bytes address)
    let expected_encoded = hex::decode(concat!(
        "000000000000000000000000",                 // 12 bytes padding
        "f39fd6e51aad88f6f4ce6ab8827279cfffb92266"  // 20 bytes address
    ))
    .unwrap();

    // Create test address
    let test_value = Address::from(hex!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Address encoding doesn't match expected value"
    );

    // Verify metadata for fixed-size types
    let (offset, length) = SolidityABI::<Address>::partial_decode(&encoded, 0).unwrap();
    assert_eq!(
        offset, 0,
        "Fixed-size types should be encoded inline (offset = 0)"
    );
    assert_eq!(
        length, 32,
        "Fixed-size types should always occupy full 32-byte words"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<Address>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}
#[test]
fn test_vector_solidity_simple() {
    // Define expected encoding for simple vector:
    // - First 32 bytes: offset (32)
    // - Next 32 bytes: length (3)
    // - Following bytes: elements, each padded to 32 bytes
    let expected_encoded = hex::decode(concat!(
        "0000000000000000000000000000000000000000000000000000000000000020", // offset
        "0000000000000000000000000000000000000000000000000000000000000003", // length
        "0000000000000000000000000000000000000000000000000000000000000001", // value[0]
        "0000000000000000000000000000000000000000000000000000000000000002", // value[1]
        "0000000000000000000000000000000000000000000000000000000000000003"  // value[2]
    ))
    .unwrap();

    // Create test vector
    let test_value: Vec<u32> = vec![1, 2, 3];

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Vector encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<Vec<u32>>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_vector_solidity_nested() {
    // Define expected encoding for nested vector:
    // - First 32 bytes: main offset (32)
    // - Next 32 bytes: array length (2)
    // - Next 32 bytes: offset of first inner array (64)
    // - Next 32 bytes: offset of second inner array (192)
    // For first inner array:
    // - 32 bytes: length (3)
    // - 3 * 32 bytes: values
    // For second inner array:
    // - 32 bytes: length (2)
    // - 2 * 32 bytes: values
    let expected_encoded = hex::decode(concat!(
        // Main array header
        "0000000000000000000000000000000000000000000000000000000000000020", // offset
        "0000000000000000000000000000000000000000000000000000000000000002", // length
        "0000000000000000000000000000000000000000000000000000000000000040", // offset of vec[0]
        "00000000000000000000000000000000000000000000000000000000000000c0", // offset of vec[1]
        // First inner array
        "0000000000000000000000000000000000000000000000000000000000000003", // length
        "0000000000000000000000000000000000000000000000000000000000000001", // vec[0][0]
        "0000000000000000000000000000000000000000000000000000000000000002", // vec[0][1]
        "0000000000000000000000000000000000000000000000000000000000000003", // vec[0][2]
        // Second inner array
        "0000000000000000000000000000000000000000000000000000000000000002", // length
        "0000000000000000000000000000000000000000000000000000000000000004", // vec[1][0]
        "0000000000000000000000000000000000000000000000000000000000000005"  // vec[1][1]
    ))
    .unwrap();

    // Create test nested vector
    let test_value: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Nested vector encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<Vec<Vec<u32>>>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_vector_solidity_empty() {
    // Define expected encoding for empty vector:
    // - First 32 bytes: offset (32)
    // - Next 32 bytes: length (0)
    // - No additional data since length is 0
    let expected_encoded = hex::decode(concat!(
        "0000000000000000000000000000000000000000000000000000000000000020", // offset to data
        "0000000000000000000000000000000000000000000000000000000000000000"  // length (0)
    ))
    .unwrap();

    // Create empty vector
    let test_value: Vec<u32> = vec![];

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Empty vector encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<Vec<u32>>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_vector_wasm_nested() {
    // Define expected encoding for FluentABI nested vector format:
    // Header (main array):
    // - 4 bytes: number of vectors (3)
    // - 4 bytes: offset to first vector (12)
    // - 4 bytes: offset to second vector (76)
    // First vector [1,2,3]:
    // - 4 bytes: length (3)
    // - 4 bytes: relative offset (36)
    // - 12 bytes: data (3 * 4 bytes)
    // Second vector [4,5]:
    // - 4 bytes: length (2)
    // - 4 bytes: relative offset (48)
    // - 8 bytes: data (2 * 4 bytes)
    // Third vector [6,7,8,9,10]:
    // - 4 bytes: length (5)
    // - 4 bytes: relative offset (56)
    // - 20 bytes: data (5 * 4 bytes)
    let expected_encoded = hex::decode(concat!(
        // Main array header
        "03000000", // length (3)
        "0c000000", // offset to first vector
        "4c000000", // offset to second vector
        // First vector [1,2,3]
        "03000000", // length
        "24000000", // relative offset
        "0c000000", // data offset
        // Second vector [4,5]
        "02000000", // length
        "30000000", // relative offset
        "08000000", // data offset
        // Third vector [6,7,8,9,10]
        "05000000", // length
        "38000000", // relative offset
        "14000000", // data offset
        // Data sections
        "01000000", // 1
        "02000000", // 2
        "03000000", // 3
        "04000000", // 4
        "05000000", // 5
        "06000000", // 6
        "07000000", // 7
        "08000000", // 8
        "09000000", // 9
        "0a000000"  // 10
    ))
    .unwrap();

    // Create test nested vector
    let test_value: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5], vec![6, 7, 8, 9, 10]];

    // Encode and verify
    let mut buf = BytesMut::new();
    FluentABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Nested vector encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = FluentABI::<Vec<Vec<u32>>>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_bytes_empty_solidity() {
    // Define expected encoding for empty bytes:
    // - First 32 bytes: offset (32)
    // - Next 32 bytes: length (0)
    // - No data since length is 0
    let expected_encoded = hex::decode(concat!(
        "0000000000000000000000000000000000000000000000000000000000000020", // offset
        "0000000000000000000000000000000000000000000000000000000000000000"  // length (0)
    ))
    .unwrap();

    // Create empty bytes
    let test_value = Bytes::new();

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Empty bytes encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = SolidityABI::<Bytes>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}
#[test]
fn test_vector_solidity_partial_decode() {
    // Define expected encoding for vector with 5 elements:
    // - First 32 bytes: offset (32)
    // - Next 32 bytes: length (5)
    // - Following 32 bytes * 5: elements [1,2,3,4,5]
    let expected_encoded = hex::decode(concat!(
        "0000000000000000000000000000000000000000000000000000000000000020", // offset
        "0000000000000000000000000000000000000000000000000000000000000005", // length
        "0000000000000000000000000000000000000000000000000000000000000001", // value[0]
        "0000000000000000000000000000000000000000000000000000000000000002", // value[1]
        "0000000000000000000000000000000000000000000000000000000000000003", // value[2]
        "0000000000000000000000000000000000000000000000000000000000000004", // value[3]
        "0000000000000000000000000000000000000000000000000000000000000005"  // value[4]
    ))
    .unwrap();

    // Create test vector
    let test_value: Vec<u32> = vec![1, 2, 3, 4, 5];

    // Encode and verify
    let mut buf = BytesMut::new();
    SolidityABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Vector encoding doesn't match expected value"
    );

    // Test partial decoding - should return (offset, length)
    let (offset, length) = SolidityABI::<Vec<u32>>::partial_decode(&encoded, 0).unwrap();

    assert_eq!(offset, 32, "Vector data should start at offset 32");
    assert_eq!(length, 5, "Vector should contain 5 elements");

    // Optional: Verify that full decoding still works
    let decoded = SolidityABI::<Vec<u32>>::decode(&encoded, 0).unwrap();
    assert_eq!(
        decoded, test_value,
        "Full decoding should still work after partial decode"
    );
}

#[test]
fn test_vector_wasm_partial_decode() {
    // Define expected encoding for FluentABI vector:
    // - First 4 bytes: length of vector (5)
    // - Next 4 bytes: offset to data (12)
    // - Next 4 bytes: size of data in bytes (20)
    // - Following bytes: actual data (5 * 4 bytes)
    let expected_encoded = hex::decode(concat!(
        "05000000", // length (5)
        "0c000000", // offset to data (12)
        "14000000", // size of data in bytes (20)
        "01000000", // value[0]
        "02000000", // value[1]
        "03000000", // value[2]
        "04000000", // value[3]
        "05000000"  // value[4]
    ))
    .unwrap();

    // Create test vector
    let test_value: Vec<u32> = vec![1, 2, 3, 4, 5];

    // Encode and verify
    let mut buf = BytesMut::new();
    FluentABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "FluentABI vector encoding doesn't match expected value"
    );

    // Test partial decoding - should return (offset, data_size)
    // Note: We skip first 4 bytes (length) to get to the offset
    let (data_offset, data_size) = FluentABI::<Vec<u32>>::partial_decode(&encoded, 4).unwrap();

    assert_eq!(data_offset, 12, "Vector data should start at offset 12");
    assert_eq!(
        data_size, 20,
        "Vector data should be 20 bytes (5 elements * 4 bytes)"
    );

    // Verify first two elements in the data section
    assert_eq!(
        &encoded[data_offset..data_offset + 8],
        &[1, 0, 0, 0, 2, 0, 0, 0],
        "First two u32 values should be encoded correctly"
    );

    // Optional: Verify full decoding still works
    let decoded = FluentABI::<Vec<u32>>::decode(&encoded, 0).unwrap();
    assert_eq!(
        decoded, test_value,
        "Full decoding should still work after partial decode"
    );
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
    original.insert(1, HashMap::from([(5, 6)]));
    original.insert(2, HashMap::from([(7, 8)]));

    let mut buf = BytesMut::new();
    SolidityABI::encode(&original, &mut buf, 0).unwrap();

    let encoded = buf.freeze();

    println!("Encoded Map: {:?}", hex::encode(&encoded));

    let expected_encoded = "000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000005000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000008";

    assert_eq!(hex::encode(&encoded), expected_encoded, "Encoding mismatch");
    let decoded = SolidityABI::<HashMap<u32, HashMap<u32, u32>>>::decode(&&encoded[..], 0).unwrap();
    println!("Decoded Map: {:?}", decoded);

    assert_eq!(decoded, original);
}
#[test]
fn test_map_wasm_simple() {
    // Define expected encoding for FluentABI simple map:
    // Header:
    // - length (4 bytes): number of pairs
    // - data_offset (4 bytes): offset to keys-values area
    // - keys_size (4 bytes): total size of keys section
    // - values_offset (4 bytes): offset to values section
    let expected_encoded = hex::decode(concat!(
        // Header
        "03000000", // length (3 pairs)
        "14000000", // data_offset (20)
        "0c000000", // keys_size (12)
        "20000000", // values_offset (32)
        "0c000000", // values_size (12)
        // Keys (sorted)
        "03000000", // key = 3
        "64000000", // key = 100
        "e8030000", // key = 1000
        // Values (in same order as keys)
        "05000000", // value = 5
        "14000000", // value = 20
        "3c000000"  // value = 60
    ))
    .unwrap();

    // Create test data with multiple key-value pairs
    let test_value = HashMap::from([(100, 20), (3, 5), (1000, 60)]);

    // Encode and verify
    let mut buf = BytesMut::new();
    FluentABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Simple HashMap encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = FluentABI::<HashMap<u32, u32>>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_map_wasm_nested() {
    // Define expected encoding for FluentABI nested map:
    // Main header:
    // - length (4 bytes): number of outer pairs
    // - data_offset (4 bytes): offset to keys-values area
    // - keys_size (4 bytes): size of outer keys section
    // - values_offset (4 bytes): offset to outer values section
    // For each inner map:
    // - Similar structure with its own header and data sections
    let expected_encoded = hex::decode(concat!(
        // Outer map header
        "02000000", // length (2 pairs)
        "14000000", // data_offset (20)
        "08000000", // keys_size (8)
        "1c000000", // values_offset (28)
        "38000000", // nested data offset
        // Outer keys
        "01000000", // key[0] = 1
        "02000000", // key[1] = 2
        // First inner map {5: 6}
        "01000000", // length (1)
        "28000000", // data_offset
        "04000000", // keys_size
        "2c000000", // values_offset
        "04000000", // additional offset
        // Second inner map {7: 8}
        "01000000", // length (1)
        "30000000", // data_offset
        "04000000", // keys_size
        "34000000", // values_offset
        "04000000", // additional offset
        // Inner map values
        "05000000", // key = 5
        "06000000", // value = 6
        "07000000", // key = 7
        "08000000"  // value = 8
    ))
    .unwrap();

    // Create test nested HashMap
    let test_value = HashMap::from([(1, HashMap::from([(5, 6)])), (2, HashMap::from([(7, 8)]))]);

    // Encode and verify
    let mut buf = BytesMut::new();
    FluentABI::encode(&test_value, &mut buf, 0).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Nested HashMap encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = FluentABI::<HashMap<u32, HashMap<u32, u32>>>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}
