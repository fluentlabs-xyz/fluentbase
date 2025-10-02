/// Extend the SHA-256 message schedule in place.
/// `w[0..16]` must be pre-filled with the message block (big-endian words).
/// Fills `w[16..64]` according to the standard recurrence.
use crate::RuntimeContext;
use rwasm::{Store, TrapCode, Value};

pub fn syscall_hashing_sha256_extend_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let w_ptr: u32 = params[0].i32().unwrap() as u32;
    let mut w = [0u32; 64];
    let mut block = [0u8; 64];
    ctx.memory_read(w_ptr as usize, &mut block)?;
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4 + 0],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    syscall_hashing_sha256_extend_impl(&mut w);
    // Write W[0..64] back as big-endian words (256 bytes)
    let mut out = [0u8; 64 * 4];
    for i in 0..64 {
        out[i * 4..i * 4 + 4].copy_from_slice(&w[i].to_be_bytes());
    }
    ctx.memory_write(w_ptr as usize, &out)?;
    Ok(())
}

pub fn syscall_hashing_sha256_extend_impl(w: &mut [u32; 64]) {
    for i in 16..64 {
        let x = w[i - 15];
        let y = w[i - 2];
        let s0 = x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3);
        let s1 = y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
}
