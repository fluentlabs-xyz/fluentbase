#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate fluentbase_codec as codec;

use byteorder::{BigEndian, LittleEndian};
use bytes::BytesMut;
use codec::optimized::{encoder::Encoder, CompactABI, SolidityABI};

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn main() {
    use bytes::buf;

    let mut buf = [0u8; 1024];

    let mut buf = buf.as_mut_slice();

    // Test u32 encoding (CompactABI - LE, align 4)
    let test_u32: u32 = 0x12345678;
    <u32 as Encoder<LittleEndian, 4, false>>::encode(&test_u32, &mut buf, None).unwrap();
    // buf.clear();
    core::hint::black_box(&mut *buf);

    // Test u32 encoding (SolidityABI - BE, align 32)
    <u32 as Encoder<BigEndian, 32, true>>::encode(&test_u32, &mut buf, None).unwrap();
    // buf.clear();
    core::hint::black_box(&mut *buf);

    // Test bool encoding
    let test_bool = true;
    <bool as Encoder<LittleEndian, 4, false>>::encode(&test_bool, &mut buf, None).unwrap();
    // buf.clear();
    core::hint::black_box(&mut *buf);

    // Test array encoding
    let test_array: [u8; 5] = [1, 2, 3, 4, 5];
    <[u8; 5] as Encoder<LittleEndian, 4, false>>::encode(&test_array, &mut buf, None).unwrap();
    // buf.clear();
    core::hint::black_box(&mut *buf);

    // Test u64 encoding
    let test_u64: u64 = 0x123456789ABCDEF0;
    <u64 as Encoder<LittleEndian, 8, false>>::encode(&test_u64, &mut buf, None).unwrap();
    // buf.clear();
    core::hint::black_box(&mut *buf);

    // Test Option<u32> encoding
    let test_option: Option<u32> = Some(42);
    <Option<u32> as Encoder<LittleEndian, 4, false>>::encode(&test_option, &mut buf, None).unwrap();
    // buf.clear();
    core::hint::black_box(&mut *buf);

    // Test using high-level API
    CompactABI::<u32>::encode(&test_u32, &mut buf).unwrap();
    // buf.clear();
    core::hint::black_box(&mut *buf);

    SolidityABI::<u32>::encode(&test_u32, &mut buf).unwrap();
    // buf.clear();
    core::hint::black_box(&mut *buf);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("This example should be compiled for wasm32-unknown-unknown target");
}
