use super::*;

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
    SolidityABI::encode(&test_value, &mut buf).unwrap();
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
    SolidityABI::encode(&test_value, &mut buf).unwrap();
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
    CompactABI::encode(&test_value, &mut buf).unwrap();
    let encoded = buf.freeze();

    assert_eq!(
        encoded.to_vec(),
        expected_encoded,
        "Tuple encoding doesn't match expected value"
    );

    // Verify round-trip encoding/decoding
    let decoded = CompactABI::<(Bytes, Address)>::decode(&encoded, 0).unwrap();
    assert_eq!(decoded, test_value, "Round-trip encoding/decoding failed");
}

#[test]
fn test_nested_tuple_static() {
    sol!(
        contract TupleContract {
            struct Point {
                uint64 x;
            }

            struct ComplexPoint {
                Point p;
                uint64 y;
            }
        }
    );

    let point_tuple = (42u64,);
    let complex_point_tuple = ((42u64,), 24u64);

    let point_sol = TupleContract::Point { x: 42u64 };

    let complex_point_sol = TupleContract::ComplexPoint {
        p: TupleContract::Point { x: 42u64 },
        y: 24u64,
    };

    {
        let point_expected = point_sol.abi_encode();

        let mut buf = BytesMut::new();
        SolidityABI::encode(&point_tuple, &mut buf).unwrap();
        let encoded = buf.freeze();

        assert_eq!(
            hex::encode(&point_expected),
            hex::encode(&encoded),
            "Simple tuple encoding mismatch with expected data"
        );

        let decoded = SolidityABI::<(u64,)>::decode(&encoded, 0).unwrap();
        assert_eq!(
            decoded, point_tuple,
            "Round-trip encoding/decoding failed for simple static tuple"
        );
    }

    {
        let complex_point_expected = complex_point_sol.abi_encode();

        let mut buf = BytesMut::new();
        SolidityABI::encode(&complex_point_tuple, &mut buf).unwrap();
        let encoded = buf.freeze();

        assert_eq!(
            hex::encode(&complex_point_expected),
            hex::encode(&encoded),
            "Complex tuple encoding mismatch with expected data"
        );

        let decoded = SolidityABI::<((u64,), u64)>::decode(&encoded, 0).unwrap();
        assert_eq!(
            decoded, complex_point_tuple,
            "Round-trip encoding/decoding failed for nested static tuple"
        );
    }
}

#[test]
fn test_nested_tuple_dynamic() {
    sol!(
        contract SmallNestedContract {
            struct Small {
                bool bool_val;
                bytes bytes_val;
                uint32[] vec_val;
            }

            struct Nested {
                Small nested_struct;
                bytes32[2] fixed_bytes;
                uint32 uint_val;
                uint32[] vec_val;
            }
        }
    );

    let small_tuple = (true, Bytes::from(vec![1, 2, 3, 4, 5]), vec![10u32, 20, 30]);

    let nested_tuple = (
        small_tuple.clone(),
        [
            FixedBytes::<32>::from_slice(&[0x11; 32]),
            FixedBytes::<32>::from_slice(&[0x11; 32]),
        ],
        42u32,
        vec![100u32, 200, 300],
    );

    let sol_small_value = SmallNestedContract::Small {
        bool_val: small_tuple.0,
        bytes_val: small_tuple.1.clone(),
        vec_val: small_tuple.2.clone(),
    };

    let sol_nested_value = SmallNestedContract::Nested {
        nested_struct: SmallNestedContract::Small {
            bool_val: nested_tuple.0 .0,
            bytes_val: nested_tuple.0 .1.clone(),
            vec_val: nested_tuple.0 .2.clone(),
        },
        fixed_bytes: nested_tuple.1.clone(),
        uint_val: nested_tuple.2,
        vec_val: nested_tuple.3.clone(),
    };

    let sol_small_encoded = sol_small_value.abi_encode();
    let sol_nested_encoded = sol_nested_value.abi_encode();

    {
        let mut buf = BytesMut::new();
        SolidityABI::encode(&small_tuple, &mut buf).unwrap();
        let small_encoded = buf.freeze();

        assert_eq!(
            hex::encode(&sol_small_encoded),
            hex::encode(&small_encoded),
            "Encoding mismatch for small tuple"
        );

        let decoded = SolidityABI::<(bool, Bytes, Vec<u32>)>::decode(&small_encoded, 0).unwrap();
        assert_eq!(
            decoded, small_tuple,
            "Round-trip encoding/decoding failed for small dynamic tuple"
        );
    }

    {
        let mut buf = BytesMut::new();
        SolidityABI::encode(&nested_tuple, &mut buf).unwrap();
        let encoded = buf.freeze();

        assert_eq!(
            hex::encode(&sol_nested_encoded),
            hex::encode(&encoded),
            "Encoding mismatch with expected data"
        );

        let decoded =
            SolidityABI::<((bool, Bytes, Vec<u32>), [FixedBytes<32>; 2], u32, Vec<u32>)>::decode(
                &encoded, 0,
            )
            .unwrap();
        assert_eq!(decoded, nested_tuple, "Round-trip encoding/decoding failed");
    }
}

#[test]
fn test_single_tuple_solidity() {
    // Test for dynamic single value tuple
    {
        sol!(
            contract DynamicValueContract {
                struct DynamicValue {
                    bytes value;
                }
            }
        );

        let dynamic_tuple = (Bytes::from(vec![1, 2, 3, 4, 5]),);
        let sol_dynamic_value = DynamicValueContract::DynamicValue {
            value: dynamic_tuple.0.clone(),
        };

        let sol_dynamic_encoded = sol_dynamic_value.abi_encode();

        let mut buf = BytesMut::new();
        SolidityABI::encode(&dynamic_tuple, &mut buf).unwrap();
        let dynamic_encoded = buf.freeze();

        assert_eq!(
            hex::encode(&sol_dynamic_encoded),
            hex::encode(&dynamic_encoded),
            "Encoding mismatch for single dynamic tuple"
        );

        let decoded = SolidityABI::<(Bytes,)>::decode(&dynamic_encoded, 0).unwrap();
        assert_eq!(
            decoded, dynamic_tuple,
            "Round-trip encoding/decoding failed for single dynamic tuple"
        );
    }

    // Test for static single value tuple
    {
        sol!(
            contract StaticValueContract {
                struct StaticValue {
                    uint64 value;
                }
            }
        );

        let static_tuple = (42u64,);
        let sol_static_value = StaticValueContract::StaticValue {
            value: static_tuple.0,
        };

        let sol_static_encoded = sol_static_value.abi_encode();

        let mut buf = BytesMut::new();
        SolidityABI::encode(&static_tuple, &mut buf).unwrap();
        let static_encoded = buf.freeze();

        assert_eq!(
            hex::encode(&sol_static_encoded),
            hex::encode(&static_encoded),
            "Encoding mismatch for single static tuple"
        );

        let decoded = SolidityABI::<(u64,)>::decode(&static_encoded, 0).unwrap();
        assert_eq!(
            decoded, static_tuple,
            "Round-trip encoding/decoding failed for single static tuple"
        );
    }
}
