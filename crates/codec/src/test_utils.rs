use crate::{
    encoder::{is_big_endian, read_u32_aligned, Encoder, FluentABI, SolidityABI},
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
