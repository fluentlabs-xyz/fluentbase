use crate::encoder::{is_big_endian, read_u32_aligned};
use byteorder::ByteOrder;

pub(crate) fn print_bytes<B: ByteOrder, const ALIGN: usize>(buf: &[u8]) {
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
