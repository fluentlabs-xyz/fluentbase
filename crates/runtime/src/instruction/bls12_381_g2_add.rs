use crate::RuntimeContext;
use blstrs::{G2Affine, G2Projective};
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallBls12381G2Add;

impl SyscallBls12381G2Add {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let q_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; 192];
        caller.memory_read(p_ptr, &mut p)?;

        let mut q = [0u8; 192];
        caller.memory_read(q_ptr, &mut q)?;

        Self::fn_impl(&mut p, &q);
        caller.memory_write(p_ptr, &p)?;
        Ok(())
    }

    pub fn fn_impl(p: &mut [u8; 192], q: &[u8; 192]) {
        // p, q layout: x0||x1||y0||y1, each limb 48 bytes little-endian
        // Convert to blstrs uncompressed big-endian bytes with c0/c1 swapped, add, then convert back.
        let mut a_be = [0u8; 192];
        let mut b_be = [0u8; 192];

        // Helper: copy LE limb to BE
        let mut limb = [0u8; 48];

        // For blstrs, Fp2 is serialized as c0||c1 (each 48B BE).
        // Our runtime uses x0||x1 (each 48B LE). If runtime x0 corresponds to blstrs c1,
        // swap the order when building BE buffers: [c0<=x1, c1<=x0]. Same for y.
        // Build a
        // x: c0 <= x1, c1 <= x0
        limb.copy_from_slice(&p[48..96]); // x1 LE
        limb.reverse();
        a_be[0..48].copy_from_slice(&limb); // c0
        limb.copy_from_slice(&p[0..48]); // x0 LE
        limb.reverse();
        a_be[48..96].copy_from_slice(&limb); // c1
                                             // y: c0 <= y1, c1 <= y0
        limb.copy_from_slice(&p[144..192]); // y1 LE
        limb.reverse();
        a_be[96..144].copy_from_slice(&limb); // c0
        limb.copy_from_slice(&p[96..144]); // y0 LE
        limb.reverse();
        a_be[144..192].copy_from_slice(&limb); // c1

        // Build b
        limb.copy_from_slice(&q[48..96]); // x1 LE
        limb.reverse();
        b_be[0..48].copy_from_slice(&limb); // c0
        limb.copy_from_slice(&q[0..48]); // x0 LE
        limb.reverse();
        b_be[48..96].copy_from_slice(&limb); // c1
        limb.copy_from_slice(&q[144..192]); // y1 LE
        limb.reverse();
        b_be[96..144].copy_from_slice(&limb); // c0
        limb.copy_from_slice(&q[96..144]); // y0 LE
        limb.reverse();
        b_be[144..192].copy_from_slice(&limb); // c1

        // Parse into affine points (validated), add in projective, and convert back to affine
        let a_aff = G2Affine::from_uncompressed(&a_be).unwrap();
        let b_aff = G2Affine::from_uncompressed(&b_be).unwrap();
        let a = G2Projective::from(a_aff);
        let b = G2Projective::from(b_aff);
        let sum = &a + &b;
        let sum_aff = G2Affine::from(sum);

        // Serialize to BE uncompressed and convert BE -> LE limbs back into p (swap back)
        let sum_be = sum_aff.to_uncompressed();
        // x0 <= c1, x1 <= c0
        limb.copy_from_slice(&sum_be[48..96]); // c1
        limb.reverse();
        p[0..48].copy_from_slice(&limb); // x0 LE
        limb.copy_from_slice(&sum_be[0..48]); // c0
        limb.reverse();
        p[48..96].copy_from_slice(&limb); // x1 LE
                                          // y0 <= c1, y1 <= c0
        limb.copy_from_slice(&sum_be[144..192]); // c1
        limb.reverse();
        p[96..144].copy_from_slice(&limb); // y0 LE
        limb.copy_from_slice(&sum_be[96..144]); // c0
        limb.reverse();
        p[144..192].copy_from_slice(&limb); // y1 LE
    }
}
