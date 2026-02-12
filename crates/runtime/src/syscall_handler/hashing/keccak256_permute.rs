use crate::RuntimeContext;
use rwasm::{StoreTr, TrapCode, Value};

pub(crate) const STATE_SIZE: u32 = 25;

// The permutation state is 25 u64's. Our word size is 32 bits, so it is 50 words.
pub const STATE_NUM_WORDS: u32 = STATE_SIZE * 2;

#[repr(C, align(8))]
struct Aligned200([u8; 200]);

impl Aligned200 {
    /// Consume self and reinterpret bytes as 25 little-endian u64 lanes.
    #[inline]
    pub fn into_lanes_le(self) -> [u64; 25] {
        // Compile-time sanity checks
        const _: () = assert!(size_of::<[u8; 200]>() == size_of::<[u64; 25]>());
        #[cfg(not(target_endian = "little"))]
        const _: () = panic!("into_lanes_le requires little-endian platform");
        // All bit patterns are valid u64, and sizes match.
        // Endianness: this does NOT swap bytes; your bytes must already be LE per lane.
        unsafe { core::mem::transmute::<[u8; 200], [u64; 25]>(self.0) }
    }

    /// Produce a wrapper from lanes (inverse of above).
    #[inline]
    pub fn from_lanes_le(lanes: [u64; 25]) -> Self {
        #[cfg(not(target_endian = "little"))]
        const _: () = panic!("from_lanes_le requires little-endian platform");

        let bytes: [u8; 200] = unsafe { core::mem::transmute(lanes) };
        Self(bytes)
    }
}

pub fn syscall_hashing_keccak256_permute_handler(
    ctx: &mut impl StoreTr<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let state_ptr = params[0].i32().unwrap() as u32;
    let mut state = [0u8; 200];
    ctx.memory_read(state_ptr as usize, &mut state)?;
    let mut state = Aligned200(state).into_lanes_le();
    syscall_hashing_keccak256_permute_impl(&mut state);
    let state = Aligned200::from_lanes_le(state);
    ctx.memory_write(state_ptr as usize, &state.0)?;
    Ok(())
}

pub fn syscall_hashing_keccak256_permute_impl(state: &mut [u64; 25]) {
    use tiny_keccak::keccakf;
    keccakf(state);
}

#[cfg(test)]
mod tests {
    use crate::syscall_handler::syscall_hashing_keccak256_permute_impl;
    use fluentbase_types::hex;

    const RATE: usize = 136; // 1088 bits
    const LANES: usize = 25; // 25 * 8 = 200 bytes

    pub fn tiny_keccak256(inp: &[u8]) -> [u8; 32] {
        let mut s = [0u64; 25];
        let mut i = 0;
        // absorb full blocks
        while i + RATE <= inp.len() {
            for (j, &b) in inp[i..i + RATE].iter().enumerate() {
                s[j / 8] ^= (b as u64) << ((j % 8) * 8);
            }
            syscall_hashing_keccak256_permute_impl(&mut s);
            i += RATE;
        }
        // last block + padding (Keccak: 0x01 ... 0x80)
        let r = inp.len() - i;
        for (j, &b) in inp[i..].iter().enumerate() {
            s[j / 8] ^= (b as u64) << ((j % 8) * 8);
        }
        s[r / 8] ^= 1u64 << ((r % 8) * 8);
        s[(RATE - 1) / 8] ^= 0x80u64 << (((RATE - 1) % 8) * 8);
        // permute
        syscall_hashing_keccak256_permute_impl(&mut s);
        // squeeze 32 bytes
        let mut out = [0u8; 32];
        for k in 0..32 {
            out[k] = ((s[k / 8] >> ((k % 8) * 8)) & 0xFF) as u8;
        }
        out
    }

    #[test]
    fn test_keccak256_permute() {
        let hash = tiny_keccak256("Hello, World".as_bytes());
        assert_eq!(
            hash,
            hex!("a04a451028d0f9284ce82243755e245238ab1e4ecf7b9dd8bf4734d9ecfd0529")
        );
        let hash = tiny_keccak256(&[]);
        assert_eq!(
            hash,
            hex!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
        );
        let hash = tiny_keccak256("abc".as_bytes());
        assert_eq!(
            hash,
            hex!("4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45")
        );
    }

    fn ref_keccak256(data: &[u8]) -> [u8; 32] {
        use tiny_keccak::{Hasher, Keccak};
        let mut hasher = Keccak::v256(); // Ethereum Keccak-256 (pad10*1, domain 0x01)
        hasher.update(data);
        let mut out = [0u8; 32];
        hasher.finalize(&mut out);
        out
    }

    #[test]
    fn corner_lengths_near_rate() {
        // RATE = 136. Hit the boundaries around padding.
        for &len in &[
            0usize, 1, 2, 3, 7, 8, 15, 16, 31, 32, 63, 64, 127, 135, 136, 137,
        ] {
            let msg: Vec<u8> = (0..len as u64)
                .map(|x| (x as u8).wrapping_mul(0x9d))
                .collect();
            assert_eq!(tiny_keccak256(&msg), ref_keccak256(&msg), "len={}", len);
        }
    }

    #[test]
    fn unicode_bytes() {
        // Non-ASCII payloads to catch any stray UTF handling assumptions.
        let s1 = "Привет, мир! 👋🌍".as_bytes();
        let s2 = "ちりも積もれば山となる".as_bytes();
        let s3 = "مرحبا بالعالم".as_bytes();
        for (i, m) in [s1, s2, s3].into_iter().enumerate() {
            assert_eq!(tiny_keccak256(m), ref_keccak256(m), "unicode idx={}", i);
        }
    }

    #[test]
    fn long_messages() {
        // Large messages: near-rate tails and multi-MiB to shake the loop
        let mut m1 = vec![0u8; 10_000]; // 10 KB zeros
        for (i, b) in m1.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(17);
        }
        assert_eq!(tiny_keccak256(&m1), ref_keccak256(&m1));

        // 1,000,000 'a' (classic torture test but not too slow)
        let m2 = vec![b'a'; 1_000_000];
        assert_eq!(tiny_keccak256(&m2), ref_keccak256(&m2));
    }

    #[test]
    fn every_tail_len_0_to_200() {
        // Brutal: fixed 512 bytes + all possible tail lengths up to 200.
        let head: Vec<u8> = (0..512u16).map(|x| (x as u8).wrapping_mul(73)).collect();
        for tail_len in 0..=200 {
            let mut m = head.clone();
            m.extend((0..tail_len as u16).map(|x| (x as u8).wrapping_add(5)));
            assert_eq!(
                tiny_keccak256(&m),
                ref_keccak256(&m),
                "tail_len={}",
                tail_len
            );
        }
    }
}
