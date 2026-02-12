use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

pub fn syscall_hashing_sha256_compress_handler(
    ctx: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let h_ptr = params[0].i32().unwrap() as usize; // 32 bytes at state_ptr
    let w_ptr = params[1].i32().unwrap() as usize; // 256 bytes at w_ptr (W[0..63] as BE words)

    // --- Read chaining state H[0..7] (32 bytes, BE) ---
    let mut h_be = [0u8; 32];
    ctx.memory_read(h_ptr, &mut h_be)?;
    let mut state = [0u32; 8];
    for i in 0..8 {
        state[i] = u32::from_le_bytes([
            h_be[i * 4 + 0],
            h_be[i * 4 + 1],
            h_be[i * 4 + 2],
            h_be[i * 4 + 3],
        ]);
    }

    // --- Read W[0..63] (256 bytes, each word BE) ---
    let mut w_be = [0u8; 64 * 4];
    ctx.memory_read(w_ptr, &mut w_be)?;
    let mut w = [0u32; 64];
    for i in 0..64 {
        w[i] = u32::from_le_bytes([
            w_be[i * 4 + 0],
            w_be[i * 4 + 1],
            w_be[i * 4 + 2],
            w_be[i * 4 + 3],
        ]);
    }

    // --- Compress ---
    syscall_hashing_sha256_compress_impl(&mut state, &w);

    // --- Write back H as 32 bytes, BE ---
    for i in 0..8 {
        h_be[i * 4..i * 4 + 4].copy_from_slice(&state[i].to_le_bytes());
    }
    ctx.memory_write(h_ptr, &h_be)?;

    Ok(())
}

pub fn syscall_hashing_sha256_compress_impl(state: &mut [u32; 8], w: &[u32; 64]) {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
        state[0], state[1], state[2], state[3], state[4], state[5], state[6], state[7],
    );
    for t in 0..64 {
        let t1 = h
            .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
            .wrapping_add((e & f) ^ (!e & g))
            .wrapping_add(K[t])
            .wrapping_add(w[t]);
        let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
            .wrapping_add((a & b) ^ (a & c) ^ (b & c));
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

#[cfg(test)]
mod tests {
    use super::syscall_hashing_sha256_compress_impl;
    use crate::syscall_handler::hashing::syscall_hashing_sha256_extend_impl;

    const IV: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
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
        syscall_hashing_sha256_extend_impl(&mut w);
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
            syscall_hashing_sha256_compress_impl(&mut state, &w);
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
            syscall_hashing_sha256_compress_impl(&mut state, &w);
        } else {
            // need two blocks
            let w1 = be_words_from_block(&tail);
            syscall_hashing_sha256_compress_impl(&mut state, &w1);
            let mut last = [0u8; 64];
            last[56..64].copy_from_slice(&bit_len.to_be_bytes());
            let w2 = be_words_from_block(&last);
            syscall_hashing_sha256_compress_impl(&mut state, &w2);
        }
        state
    }

    #[test]
    fn sha256_single_block_abc() {
        // Prepare the single 512-bit block for message "abc"
        let mut block = [0u8; 64];
        block[0] = b'a';
        block[1] = b'b';
        block[2] = b'c';
        block[3] = 0x80;
        // last 8 bytes is length in bits (24) in big-endian
        block[63] = 24; // 0x18

        let w = be_words_from_block(&block);
        let mut state = IV;
        syscall_hashing_sha256_compress_impl(&mut state, &w);

        // Expected SHA-256("abc") digest as 8 big-endian words
        let expected = [
            0xba7816bf, 0x8f01cfea, 0x414140de, 0x5dae2223, 0xb00361a3, 0x96177a9c, 0xb410ff61,
            0xf20015ad,
        ];
        assert_eq!(state, expected);
    }

    #[test]
    fn sha256_single_block_empty() {
        // Prepare the single 512-bit block for empty message ""
        let mut block = [0u8; 64];
        block[0] = 0x80;
        // length is zero, so last 8 bytes are already zeros
        let w = be_words_from_block(&block);
        let mut state = IV;
        syscall_hashing_sha256_compress_impl(&mut state, &w);

        // Expected SHA-256("") digest
        let expected = [
            0xe3b0c442, 0x98fc1c14, 0x9afbf4c8, 0x996fb924, 0x27ae41e4, 0x649b934c, 0xa495991b,
            0x7852b855,
        ];
        assert_eq!(state, expected);
    }

    #[test]
    fn sha256_multi_block_standard_vector() {
        // "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        let msg = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        let state = sha256_hash_bytes(msg);
        let expected = [
            0x248d6a61, 0xd20638b8, 0xe5c02693, 0x0c3e6039, 0xa33ce459, 0x64ff2167, 0xf6ecedd4,
            0x19db06c1,
        ];
        assert_eq!(state, expected);
    }

    #[test]
    fn sha256_million_a() {
        let msg = vec![b'a'; 1_000_000];
        let state = sha256_hash_bytes(&msg);
        let expected = [
            0xcdc76e5c, 0x9914fb92, 0x81a1c7e2, 0x84d73e67, 0xf1809a48, 0xa497200e, 0x046d39cc,
            0xc7112cd0,
        ];
        assert_eq!(state, expected);
    }
}
