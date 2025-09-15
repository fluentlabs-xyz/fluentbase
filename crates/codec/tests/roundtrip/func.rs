use super::*;

#[test]
fn test_solidity_function_args_dynamic() {
    // Test that dynamic tuple encoding skips the outer offset
    let args = (Bytes::from("hello"), U256::from(42));

    let mut buf_normal = BytesMut::new();
    let mut buf_func = BytesMut::new();

    // Normal encoding (with tuple offset)
    SolidityABI::encode(&args, &mut buf_normal, 0).unwrap();

    // Function args encoding (without tuple offset)
    SolidityABI::encode_function_args(&args, &mut buf_func).unwrap();

    // Function args should be 32 bytes shorter (no tuple offset)
    assert_eq!(buf_func.len(), buf_normal.len() - 32);
    assert_eq!(&buf_func[..], &buf_normal[32..]);

    // Decode and verify
    let decoded: (Bytes, U256) = SolidityABI::decode_function_args(&buf_func).unwrap();
    assert_eq!(decoded, args);
}

#[test]
fn test_solidity_function_args_static() {
    // Test that static tuple encoding remains the same
    let args = (U256::from(100), Address::ZERO, 42u32);

    let mut buf_normal = BytesMut::new();
    let mut buf_func = BytesMut::new();

    // Both encodings should be identical for static types
    SolidityABI::encode(&args, &mut buf_normal, 0).unwrap();
    SolidityABI::encode_function_args(&args, &mut buf_func).unwrap();

    assert_eq!(buf_normal, buf_func);

    // Decode and verify
    let decoded: (U256, Address, u32) = SolidityABI::decode_function_args(&buf_func).unwrap();
    assert_eq!(decoded, args);
}

#[test]
fn test_compact_function_args_dynamic() {
    // Test CompactABI with dynamic types
    let args = (vec![1u8, 2, 3], "test".to_string());

    let mut buf_normal = BytesMut::new();
    let mut buf_func = BytesMut::new();

    // Normal encoding (with tuple offset)
    CompactABI::encode(&args, &mut buf_normal, 0).unwrap();

    // Function args encoding (without tuple offset)
    CompactABI::encode_function_args(&args, &mut buf_func).unwrap();

    // Function args should be 4 bytes shorter (no tuple offset)
    assert_eq!(buf_func.len(), buf_normal.len() - 4);
    assert_eq!(&buf_func[..], &buf_normal[4..]);

    // Decode and verify
    let decoded: (Vec<u8>, String) = CompactABI::decode_function_args(&buf_func).unwrap();
    assert_eq!(decoded, args);
}

#[test]
fn test_empty_and_single_args() {
    // Empty tuple
    let empty = ();
    let mut buf = BytesMut::new();

    SolidityABI::encode_function_args(&empty, &mut buf).unwrap();
    assert_eq!(buf.len(), 0);

    let decoded: () = SolidityABI::decode_function_args(&buf).unwrap();
    assert_eq!(decoded, empty);

    // Single dynamic arg
    let single = (Bytes::from("data"),);
    buf.clear();

    SolidityABI::encode_function_args(&single, &mut buf).unwrap();

    // Should skip the tuple wrapper offset
    let mut buf_normal = BytesMut::new();
    SolidityABI::encode(&single, &mut buf_normal, 0).unwrap();
    assert_eq!(&buf[..], &buf_normal[32..]);

    let decoded: (Bytes,) = SolidityABI::decode_function_args(&buf).unwrap();
    assert_eq!(decoded, single);
}
