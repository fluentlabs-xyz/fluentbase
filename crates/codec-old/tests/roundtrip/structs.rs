// use super::*;

// // TODO: d1r1 Investigate dynamic offset requirements for CompactABI encoding
// // Questions to address:
// // - Should dynamic types (Bytes, Vec) have offset table like in SolidityABI?
// // - How does CompactABI handle nested dynamic fields?
// // - Do we need to modify the encoding format for better WASM compatibility?
// //
// // Related references:
// // - Current test data layout: [bool][bytes_len][bytes_data][vec_len][vec_data]
// // - Compare with SolidityABI dynamic encoding pattern
// //
// // Priority: HIGH
// // Created: 2024-10-31
// #[test]
// fn test_struct_wasm() {
//     #[derive(Codec, Default, Debug, PartialEq)]
//     struct TestStruct {
//         bool_val: bool,
//         bytes_val: Bytes,
//         vec_val: Vec<u32>,
//     }
//     // Reference encoded data for CompactABI format
//     let expected_encoded = hex::decode(concat!(
//         "01000000",                 // bool_val: true
//         "18000000050000000",        // bytes_val offset and data size
//         "3000000200000000c000000",  // vec_val length, offset and data length
//         "0102030405000000",         // bytes_val data
//         "0a000000140000001e000000", // vec_val data: [10, 20, 30]
//     ))
//     .unwrap();

//     // Create test structure with sample data
//     let test_struct = TestStruct {
//         bool_val: true,
//         bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
//         vec_val: vec![10, 20, 30],
//     };

//     // Encode using CompactABI
//     let mut buf = BytesMut::new();
//     CompactABI::encode(&test_struct, &mut buf, 0).unwrap();
//     let encoded = buf.freeze();

//     // Verify encoding matches reference data
//     assert_eq!(
//         encoded.to_vec(),
//         expected_encoded,
//         "Encoding mismatch with expected data"
//     );

//     // Verify round-trip encoding/decoding
//     let decoded = CompactABI::<TestStruct>::decode(&encoded, 0).unwrap();
//     assert_eq!(decoded, test_struct, "Round-trip encoding/decoding failed");
// }

// #[test]
// fn test_complex_struct_solidity_packed() {
//     #[derive(Debug, PartialEq, Codec)]
//     struct TestStructFluent {
//         u_8: u8,
//         u_16: u16,
//         u_32: u32,
//         u_64: u64,
//         boolean: bool,
//         bytes_1: [u8; 1],
//         bytes_32: [u8; 32],
//         tuple: (u16, bool, [u8; 2], u32),
//     }

//     let original = TestStructFluent {
//         u_8: 255,
//         u_16: 65535,
//         u_32: 4294967295,
//         u_64: 18446744073709551615,
//         boolean: true,
//         bytes_1: [0xFF],
//         bytes_32: [
//             1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
// 24,             25, 26, 27, 28, 29, 30, 31, 32,
//         ],
//         tuple: (43981, true, [0xAA, 0xBB], 0xCCDDEEFF),
//     };

//     let expected_encoded = concat!(
//         "ff",               // u8 (1 byte)
//         "ffff",             // u16 (2 bytes)
//         "ffffffff",         // u32 (4 bytes)
//         "ffffffffffffffff", // u64 (8 bytes)
//         "01",               // bool (1 byte)
//         "ff",               // bytes_1 (1 byte)
//         // bytes_32 (32 bytes)
//         "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
//         // tuple:
//         "abcd",     // u16 (2 bytes)
//         "01",       // bool (1 byte)
//         "aabb",     // [u8; 2] (2 bytes)
//         "ccddeeff"  // u32 (4 bytes)
//     );

//     let mut buf = BytesMut::new();

//     SolidityPackedABI::encode(&original, &mut buf, 0).unwrap();
//     let encoded = buf.freeze();

//     assert_eq!(expected_encoded, hex::encode(&encoded));

//     let decoded: TestStructFluent = SolidityPackedABI::decode(&encoded, 0).unwrap();

//     assert_eq!(decoded, original);
// }

// mod solidity {
//     use super::*;

//     #[test]
//     fn test_one_field_struct() {
//         let expected_encoded = hex::decode(
//             
// "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000e48656c6c6f2c20576f726c642121000000000000000000000000000000000000"
//         ).unwrap();

//         #[derive(Codec, Default, Debug, PartialEq)]
//         struct TestStruct {
//             bytes_val: Bytes,
//         }

//         let test_struct = TestStruct {
//             bytes_val: Bytes::from("Hello, World!!".as_bytes()),
//         };

//         let mut buf = BytesMut::new();
//         SolidityABI::encode(&test_struct, &mut buf, 0).unwrap();
//         let encoded = buf.freeze();

//         assert_eq!(expected_encoded, encoded.to_vec());

//         let decoded = SolidityABI::<TestStruct>::decode(&&encoded[..], 0).unwrap();

//         assert_eq!(decoded, test_struct);
//     }

//     #[test]
//     fn test_multiple_fields_struct() {
//         let expected_encoded = hex::decode(
//             
// "00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000002a00000000000000000000000000000000000000000000000000000000000003e800000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000003b9aca00fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc18fffffffffffffffffffffffffffffffffffffffffffffffffffffffffff0bdc0ffffffffffffffffffffffffffffffffffffffffffffffffffffffffc46536000000000000000000000000000000000000000000000000000000000000003039000000000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000001c0000000000000000000000000000000000000000000000000000000000000000501020304050000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000000000000000000000000001e"
//         ).unwrap();

//         // Test structure definition with various Solidity types
//         #[derive(Codec, Default, Debug, PartialEq)]
//         struct TestStruct {
//             bool_val: bool,
//             u8_val: u8,
//             uint_val: (u16, u32, u64),
//             int_val: (i16, i32, i64),
//             u256_val: U256,
//             address_val: Address,
//             bytes_val: Bytes,
//             vec_val: Vec<u32>,
//         }

//         // Create test structure instance with boundary values
//         let test_struct = TestStruct {
//             bool_val: true,
//             u8_val: 42,
//             uint_val: (1000, 1_000_000, 1_000_000_000),
//             int_val: (-1000, -1_000_000, -1_000_000_000),
//             u256_val: U256::from(12345),
//             address_val: Address::repeat_byte(0xAA),
//             bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
//             vec_val: vec![10, 20, 30],
//         };

//         // Perform encoding using SolidityABI
//         let mut buf = BytesMut::new();
//         SolidityABI::encode(&test_struct, &mut buf, 0).unwrap();
//         let encoded = buf.freeze();

//         // Verify encoding matches Solidity reference
//         assert_eq!(
//             encoded.to_vec(),
//             expected_encoded,
//             "Encoding mismatch with expected data"
//         );

//         // Verify round-trip encoding/decoding
//         let decoded = SolidityABI::<TestStruct>::decode(&encoded, 0).unwrap();
//         assert_eq!(decoded, test_struct, "Round-trip encoding/decoding failed");
//     }

//     #[test]
//     fn test_nested_struct_dynamic() {
//         // Test structures with different Solidity types
//         #[derive(Codec, Default, Debug, PartialEq, Clone)]
//         struct Small {
//             bool_val: bool,
//             bytes_val: Bytes,
//             vec_val: Vec<u32>,
//         }

//         #[derive(Codec, Default, Debug, PartialEq)]
//         struct Nested {
//             nested_struct: Small,
//             fixed_bytes: [FixedBytes<32>; 2],
//             uint_val: u32,
//             vec_val: Vec<u32>,
//         }

//         sol!(
//             contract SmallNestedContract {
//                 struct Small {
//                     bool bool_val;
//                     bytes bytes_val;
//                     uint32[] vec_val;
//                 }

//                 struct Nested {
//                     Small nested_struct;
//                     bytes32[2] fixed_bytes;
//                     uint32 uint_val;
//                     uint32[] vec_val;
//                 }
//             }
//         );

//         let small_value = Small {
//             bool_val: true,
//             bytes_val: Bytes::from(vec![1, 2, 3, 4, 5]),
//             vec_val: vec![10, 20, 30],
//         };

//         let nested_value = Nested {
//             nested_struct: small_value.clone(),
//             fixed_bytes: [
//                 FixedBytes::<32>::from_slice(&[0x11; 32]),
//                 FixedBytes::<32>::from_slice(&[0x11; 32]),
//             ],
//             uint_val: 42,
//             vec_val: vec![100, 200, 300],
//         };

//         let sol_nested_value = SmallNestedContract::Nested {
//             nested_struct: SmallNestedContract::Small {
//                 bool_val: nested_value.nested_struct.bool_val.clone(),
//                 bytes_val: nested_value.nested_struct.bytes_val.clone(),
//                 vec_val: nested_value.nested_struct.vec_val.clone(),
//             },
//             fixed_bytes: nested_value.fixed_bytes.clone(),
//             uint_val: nested_value.uint_val.clone(),
//             vec_val: nested_value.vec_val.clone(),
//         };
//         let sol_nested_encoded = sol_nested_value.abi_encode();

//         let sol_small_value = SmallNestedContract::Small {
//             bool_val: small_value.bool_val.clone(),
//             bytes_val: small_value.bytes_val.clone(),
//             vec_val: small_value.vec_val.clone(),
//         };
//         let sol_small_encoded = sol_small_value.abi_encode();

//         let mut buf = BytesMut::new();
//         SolidityABI::encode(&small_value, &mut buf, 0).unwrap();
//         let small_encoded = buf.freeze();

//         assert_eq!(
//             hex::encode(&sol_small_encoded),
//             hex::encode(&small_encoded),
//             "Encoding mismatch for small struct"
//         );

//         // Encode struct using SolidityABI
//         let mut buf = BytesMut::new();
//         SolidityABI::encode(&nested_value, &mut buf, 0).unwrap();
//         let encoded = buf.freeze();

//         // Verify encoding matches Solidity reference
//         assert_eq!(
//             hex::encode(&sol_nested_encoded),
//             hex::encode(&encoded),
//             "Encoding mismatch with expected data"
//         );

//         // Verify round-trip encoding/decoding
//         let decoded = SolidityABI::<Nested>::decode(&encoded, 0).unwrap();
//         assert_eq!(decoded, nested_value, "Round-trip encoding/decoding failed");
//     }

//     #[derive(Codec, Default, Debug, PartialEq)]
//     struct Point {
//         x: u64,
//         // bytes: Bytes,
//     }

//     #[derive(Codec, Default, Debug, PartialEq)]
//     struct ComplexPoint {
//         p: Point,
//         y: u64,
//     }

//     #[test]
//     fn test_nested_struct_static() {
//         sol!(
//             contract PointContract {
//                 struct Point {
//                     uint64 x;
//                     // bytes bytes;
//                 }

//                 struct ComplexPoint {
//                     Point p;
//                     uint64 y;
//                 }
//             }
//         );

//         // Create test value using ComplexPoint struct
//         let complex_point = ComplexPoint {
//             p: Point {
//                 x: 42,
//                 // bytes: Bytes::from(vec![1, 2, 3, 4, 5]),
//                 // y: U256::from(100),
//             },
//             y: 24,
//         };

//         let point = Point {
//             x: 42,
//             // bytes: Bytes::from(vec![1, 2, 3, 4, 5]),
//         };

//         let point_sol = PointContract::Point {
//             x: complex_point.p.x.clone(),
//             // y: test_value.p.y.clone(),
//         };

//         let point_expected = point_sol.abi_encode();

//         // Create reference value using Solidity contract struct
//         let complex_point_sol = PointContract::ComplexPoint {
//             p: PointContract::Point {
//                 x: complex_point.p.x.clone(),
//                 // y: test_value.p.y.clone(),
//             },
//             y: complex_point.y.clone(),
//         };

//         // Simple test case - check if point is encoded correctly
//         {
//             let mut buf = BytesMut::new();
//             SolidityABI::encode(&point, &mut buf, 0).unwrap();
//             let encoded = buf.freeze();
//             assert_eq!(
//                 hex::encode(&point_expected),
//                 hex::encode(&encoded),
//                 "Simple point encoding mismatch with expected data"
//             );
//         }

//         // Check complex point encoding
//         {
//             // Get expected encoding from Solidity contract
//             let complex_point_expected = complex_point_sol.abi_encode();

//             // Encode the test value
//             let mut buf = BytesMut::new();
//             <ComplexPoint as Encoder<BE, 32, true, true>>::encode(&complex_point, &mut buf, 0)
//                 .unwrap();
//             // SolidityABI::encode(&complex_point, &mut buf, 0).unwrap();
//             let encoded = buf.freeze();
//             assert_eq!(
//                 hex::encode(&complex_point_expected),
//                 hex::encode(&encoded),
//                 "Encoding mismatch with expected data"
//             );
//             // Test round-trip encoding/decoding
//             let decoded = SolidityABI::<ComplexPoint>::decode(&encoded, 0).unwrap();
//             assert_eq!(
//                 decoded, complex_point,
//                 "Round-trip encoding/decoding failed for nested static struct"
//             );
//         }
//     }

//     #[test]
//     fn test_complex_struct() {
//         #[derive(Debug, PartialEq, Codec)]
//         struct TestStructFluent {
//             u_8: u8,
//             u_16: u16,
//             u_32: u32,
//             u_64: u64,
//             boolean: bool,
//             bytes_1: [u8; 1],
//             bytes_32: [u8; 32],
//             bytes: Vec<u8>,
//             dynamic_array: Vec<u64>,
//         }

//         let original = TestStructFluent {
//             u_8: 255,
//             u_16: 65535,
//             u_32: 4294967295,
//             u_64: 18446744073709551615,
//             boolean: true,
//             bytes_1: [0xFF],
//             bytes_32: [
//                 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
// 23,                 24, 25, 26, 27, 28, 29, 30, 31, 32,
//             ],
//             bytes: vec![
//                 0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC,
// 0xFD,                 0xFE, 0xFF,
//             ],
//             dynamic_array: vec![
//                 18446744073709551615,
//                 9223372036854775807,
//                 4611686018427387903,
//                 1,
//             ],
//         };

//         let expected_encoded =
// "000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000ff000000000000000000000000000000000000000000000000000000000000ffff00000000000000000000000000000000000000000000000000000000ffffffff000000000000000000000000000000000000000000000000ffffffffffffffff000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000ff000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000050000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000700000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000009000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000b000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000d000000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000000f0000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000120000000000000000000000000000000000000000000000000000000000000013000000000000000000000000000000000000000000000000000000000000001400000000000000000000000000000000000000000000000000000000000000150000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000001700000000000000000000000000000000000000000000000000000000000000180000000000000000000000000000000000000000000000000000000000000019000000000000000000000000000000000000000000000000000000000000001a000000000000000000000000000000000000000000000000000000000000001b000000000000000000000000000000000000000000000000000000000000001c000000000000000000000000000000000000000000000000000000000000001d000000000000000000000000000000000000000000000000000000000000001e000000000000000000000000000000000000000000000000000000000000001f000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000005000000000000000000000000000000000000000000000000000000000000000720000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000f000000000000000000000000000000000000000000000000000000000000000f100000000000000000000000000000000000000000000000000000000000000f200000000000000000000000000000000000000000000000000000000000000f300000000000000000000000000000000000000000000000000000000000000f400000000000000000000000000000000000000000000000000000000000000f500000000000000000000000000000000000000000000000000000000000000f600000000000000000000000000000000000000000000000000000000000000f700000000000000000000000000000000000000000000000000000000000000f800000000000000000000000000000000000000000000000000000000000000f900000000000000000000000000000000000000000000000000000000000000fa00000000000000000000000000000000000000000000000000000000000000fb00000000000000000000000000000000000000000000000000000000000000fc00000000000000000000000000000000000000000000000000000000000000fd00000000000000000000000000000000000000000000000000000000000000fe00000000000000000000000000000000000000000000000000000000000000ff0000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000ffffffffffffffff0000000000000000000000000000000000000000000000007fffffffffffffff0000000000000000000000000000000000000000000000003fffffffffffffff0000000000000000000000000000000000000000000000000000000000000001"
// ;

//         let mut buf = BytesMut::new();

//         SolidityABI::encode(&original, &mut buf, 0).unwrap();
//         let encoded = buf.freeze();

//         assert_eq!(expected_encoded, hex::encode(&encoded));

//         let decoded = SolidityABI::<TestStructFluent>::decode(&encoded, 0).unwrap();

//         assert_eq!(decoded, original);
//     }
// }
