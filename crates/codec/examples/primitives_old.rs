#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate fluentbase_codec_old as codec;


#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn main() {
    let mut buf = BytesMut::with_capacity(256);

    // Test u32 encoding (CompactABI - LE, align 4)
    let test_u32: u32 = 0x12345678;
    <u32 as Encoder<LittleEndian, 4, false, true>>::encode(&test_u32, &mut buf, 0).unwrap();
    buf.clear();

    // Test u32 encoding (SolidityABI - BE, align 32)
    <u32 as Encoder<BigEndian, 32, true, true>>::encode(&test_u32, &mut buf, 0).unwrap();
    buf.clear();

    // Test bool encoding
    let test_bool = true;
    <bool as Encoder<LittleEndian, 4, false, true>>::encode(&test_bool, &mut buf, 0).unwrap();
    buf.clear();

    // Test array encoding
    let test_array: [u8; 5] = [1, 2, 3, 4, 5];
    <[u8; 5] as Encoder<LittleEndian, 4, false, true>>::encode(&test_array, &mut buf, 0).unwrap();
    buf.clear();

    // Test u64 encoding
    let test_u64: u64 = 0x123456789ABCDEF0;
    <u64 as Encoder<LittleEndian, 8, false, true>>::encode(&test_u64, &mut buf, 0).unwrap();
    buf.clear();

    // Test Option<u32> encoding
    let test_option: Option<u32> = Some(42);
    <Option<u32> as Encoder<LittleEndian, 4, false, true>>::encode(&test_option, &mut buf, 0)
        .unwrap();
    buf.clear();

    // Test using high-level API
    CompactABI::<u32>::encode(&test_u32, &mut buf, 0).unwrap();
    buf.clear();

    SolidityABI::<u32>::encode(&test_u32, &mut buf, 0).unwrap();
    buf.clear();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("This example should be compiled for wasm32-unknown-unknown target");
}
