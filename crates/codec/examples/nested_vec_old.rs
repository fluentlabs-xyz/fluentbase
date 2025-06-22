#![cfg_attr(target_arch = "wasm32", no_std, no_main)]

extern crate alloc;
extern crate fluentbase_codec_old as codec;

use alloc::{vec, vec::Vec};
use byteorder::BigEndian;
use bytes::BytesMut;
use codec::{encoder::Encoder, SolidityABI};

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn main() {
    let mut buf = BytesMut::with_capacity(1024);

    // Simple Vec<u32>
    let simple_vec: Vec<u32> = vec![10, 20, 30];
    SolidityABI::<Vec<u32>>::encode(&simple_vec, &mut buf, 0).unwrap();
    let _decoded: Vec<u32> = SolidityABI::<Vec<u32>>::decode(&buf, 0).unwrap();
    buf.clear();

    // Nested Vec<Vec<u32>>
    let nested_vec: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];
    SolidityABI::<Vec<Vec<u32>>>::encode(&nested_vec, &mut buf, 0).unwrap();
    let _decoded: Vec<Vec<u32>> = SolidityABI::<Vec<Vec<u32>>>::decode(&buf, 0).unwrap();
    buf.clear();

    // Triple nested Vec<Vec<Vec<u32>>>
    let triple_nested: Vec<Vec<Vec<u32>>> = vec![vec![vec![1, 2], vec![3]], vec![vec![4, 5, 6]]];
    SolidityABI::<Vec<Vec<Vec<u32>>>>::encode(&triple_nested, &mut buf, 0).unwrap();
    let _decoded: Vec<Vec<Vec<u32>>> = SolidityABI::<Vec<Vec<Vec<u32>>>>::decode(&buf, 0).unwrap();
    buf.clear();

    // Mixed sizes
    let mixed_sizes: Vec<Vec<u32>> = vec![vec![], vec![42], vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]];
    SolidityABI::<Vec<Vec<u32>>>::encode(&mixed_sizes, &mut buf, 0).unwrap();
    let _decoded: Vec<Vec<u32>> = SolidityABI::<Vec<Vec<u32>>>::decode(&buf, 0).unwrap();
    buf.clear();

    // Large vectors
    let large_vec: Vec<u32> = (0..1000).collect();
    SolidityABI::<Vec<u32>>::encode(&large_vec, &mut buf, 0).unwrap();
    let _decoded: Vec<u32> = SolidityABI::<Vec<u32>>::decode(&buf, 0).unwrap();
    buf.clear();

    // Low-level API
    let example: Vec<Vec<u32>> = vec![vec![7, 8], vec![9]];
    <Vec<Vec<u32>> as Encoder<BigEndian, 32, true, false>>::encode(&example, &mut buf, 0).unwrap();
    let _decoded = <Vec<Vec<u32>> as Encoder<BigEndian, 32, true, false>>::decode(&buf, 0).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Empty for non-wasm targets
}
