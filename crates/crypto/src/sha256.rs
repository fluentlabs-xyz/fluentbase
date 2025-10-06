use crate::CryptoRuntime;
use fluentbase_types::{CryptoAPI, B256};

const IV: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

fn be_words_from_block(block: &[u8; 64]) -> [u32; 64] {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4 + 0],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    CryptoRuntime::sha256_extend(&mut w);
    w
}

fn sha256_hash_bytes(msg: &[u8]) -> [u32; 8] {
    let mut state = IV;
    // process full 64-byte blocks
    let mut i = 0usize;
    while i + 64 <= msg.len() {
        let mut block = [0u8; 64];
        block.copy_from_slice(&msg[i..i + 64]);
        let w = be_words_from_block(&block);
        CryptoRuntime::sha256_compress(&mut state, &w);
        i += 64;
    }
    // build final padding blocks
    let mut tail = [0u8; 64];
    let rem = msg.len() - i;
    tail[..rem].copy_from_slice(&msg[i..]);
    tail[rem] = 0x80;
    let bit_len = (msg.len() as u64) * 8;
    if rem <= 55 {
        // length fits in this block
        tail[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let w = be_words_from_block(&tail);
        CryptoRuntime::sha256_compress(&mut state, &w);
    } else {
        // need two blocks
        let w1 = be_words_from_block(&tail);
        CryptoRuntime::sha256_compress(&mut state, &w1);
        let mut last = [0u8; 64];
        last[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let w2 = be_words_from_block(&last);
        CryptoRuntime::sha256_compress(&mut state, &w2);
    }
    state
}

pub fn crypto_sha256(data: &[u8]) -> B256 {
    let words = sha256_hash_bytes(data);
    let mut out = [0u8; 32];
    for (i, word) in words.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    B256::from(out)
}
