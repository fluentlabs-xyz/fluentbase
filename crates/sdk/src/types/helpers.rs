use alloc::vec::Vec;

pub fn convert_endianness_fixed<const CHUNK_SIZE: usize, const ARRAY_SIZE: usize>(
    bytes: &[u8; ARRAY_SIZE],
) -> [u8; ARRAY_SIZE] {
    let reversed: [_; ARRAY_SIZE] = bytes
        .chunks_exact(CHUNK_SIZE)
        .flat_map(|chunk| chunk.iter().rev().copied())
        .enumerate()
        .fold([0u8; ARRAY_SIZE], |mut acc, (i, v)| {
            acc[i] = v;
            acc
        });
    reversed
}

pub fn convert_endianness_flexible<const CHUNK_SIZE: usize>(bytes: &[u8]) -> Vec<u8> {
    bytes
        .chunks(CHUNK_SIZE)
        .flat_map(|b| b.iter().copied().rev().collect::<Vec<u8>>())
        .collect::<Vec<u8>>()
}
