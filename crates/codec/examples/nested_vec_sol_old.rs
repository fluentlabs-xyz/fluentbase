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
    core::hint::black_box(&mut *buf);

    // Nested Vec<Vec<u32>>
    let nested_vec: Vec<Vec<u32>> = vec![vec![1, 2, 3], vec![4, 5]];
    SolidityABI::<Vec<Vec<u32>>>::encode(&nested_vec, &mut buf, 0).unwrap();
    core::hint::black_box(&mut *buf);

    // Triple nested Vec<Vec<Vec<u32>>>
    let triple_nested: Vec<Vec<Vec<u32>>> = vec![vec![vec![1, 2], vec![3]], vec![vec![4, 5, 6]]];
    SolidityABI::<Vec<Vec<Vec<u32>>>>::encode(&triple_nested, &mut buf, 0).unwrap();
    core::hint::black_box(&mut *buf);

    // Mixed sizes
    let mixed_sizes: Vec<Vec<u32>> = vec![vec![], vec![42], vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]];
    SolidityABI::<Vec<Vec<u32>>>::encode(&mixed_sizes, &mut buf, 0).unwrap();
    core::hint::black_box(&mut *buf);

    // Large vectors
    let large_vec1: Vec<u32> = (0..1000).collect();
    let large_vec2: Vec<u32> = (1000..1200).collect();
    let large_vec3: Vec<u32> = (1200..1300).collect();
    let large_vec4: Vec<u32> = (1300..1350).collect();

    let v = vec![vec![large_vec1, large_vec2, large_vec3, large_vec4]];
    SolidityABI::<Vec<Vec<Vec<u32>>>>::encode(&v, &mut buf, 0).unwrap();
    core::hint::black_box(&mut *buf);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Empty for non-wasm targets
}
