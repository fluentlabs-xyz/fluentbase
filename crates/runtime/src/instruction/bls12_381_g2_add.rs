use crate::instruction::bls12_381_consts::{FP2_LENGTH, FP_LENGTH, G2_UNCOMPRESSED_LENGTH};
use crate::instruction::bls12_381_helpers::parse_affine_g2;
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

        let mut p = [0u8; G2_UNCOMPRESSED_LENGTH];
        caller.memory_read(p_ptr, &mut p)?;

        let mut q = [0u8; G2_UNCOMPRESSED_LENGTH];
        caller.memory_read(q_ptr, &mut q)?;

        Self::fn_impl(&mut p, &q);
        caller.memory_write(p_ptr, &p)?;
        Ok(())
    }

    pub fn fn_impl(p: &mut [u8; G2_UNCOMPRESSED_LENGTH], q: &[u8; G2_UNCOMPRESSED_LENGTH]) {
        // p, q layout: x0||x1||y0||y1, each limb 48 bytes little-endian
        // Convert to blstrs uncompressed big-endian bytes with c0/c1 swapped, add, then convert back.
        let mut a_be = [0u8; G2_UNCOMPRESSED_LENGTH];
        let mut b_be = [0u8; G2_UNCOMPRESSED_LENGTH];

        // Helper: copy LE limb to BE
        let mut limb = [0u8; FP_LENGTH];

        // For blstrs, Fp2 is serialized as c0||c1 (each 48B BE).
        // Our runtime uses x0||x1 (each 48B LE). If runtime x0 corresponds to blstrs c1,
        // swap the order when building BE buffers: [c0<=x1, c1<=x0]. Same for y.

        // === Build a ===
        // x: c0 <= x1, c1 <= x0
        limb.copy_from_slice(&p[FP_LENGTH..FP2_LENGTH]); // x1 LE
        limb.reverse();
        a_be[0..FP_LENGTH].copy_from_slice(&limb); // c0
        limb.copy_from_slice(&p[0..FP_LENGTH]); // x0 LE
        limb.reverse();
        a_be[FP_LENGTH..FP2_LENGTH].copy_from_slice(&limb); // c1
                                                            // y: c0 <= y1, c1 <= y0
        limb.copy_from_slice(&p[3 * FP_LENGTH..4 * FP_LENGTH]); // y1 LE
        limb.reverse();
        a_be[FP2_LENGTH..3 * FP_LENGTH].copy_from_slice(&limb); // c0
        limb.copy_from_slice(&p[3 * FP_LENGTH..4 * FP_LENGTH]); // y0 LE
        limb.reverse();
        a_be[3 * FP_LENGTH..4 * FP_LENGTH].copy_from_slice(&limb); // c1

        // === Build b ===
        limb.copy_from_slice(&q[FP_LENGTH..FP2_LENGTH]); // x1 LE
        limb.reverse();
        b_be[0..FP_LENGTH].copy_from_slice(&limb); // c0
        limb.copy_from_slice(&q[0..FP_LENGTH]); // x0 LE
        limb.reverse();
        b_be[FP_LENGTH..FP2_LENGTH].copy_from_slice(&limb); // c1
        limb.copy_from_slice(&q[3 * FP_LENGTH..4 * FP_LENGTH]); // y1 LE
        limb.reverse();
        b_be[FP2_LENGTH..3 * FP_LENGTH].copy_from_slice(&limb); // c0
        limb.copy_from_slice(&q[3 * FP_LENGTH..4 * FP_LENGTH]); // y0 LE
        limb.reverse();
        b_be[3 * FP_LENGTH..4 * FP_LENGTH].copy_from_slice(&limb); // c1

        let a_aff = parse_affine_g2(&a_be);
        let b_aff = parse_affine_g2(&b_be);

        let sum = G2Projective::from(a_aff) + G2Projective::from(b_aff);
        let sum_aff = G2Affine::from(sum);

        // Serialize to BE uncompressed and convert BE -> LE limbs back into p (swap back)
        let sum_be = sum_aff.to_uncompressed();
        let result = sum_be;
        // x0 <= c1, x1 <= c0
        limb.copy_from_slice(&sum_be[FP_LENGTH..FP2_LENGTH]); // c1
        limb.reverse();
        p[0..FP_LENGTH].copy_from_slice(&limb); // x0 LE
        limb.copy_from_slice(&sum_be[0..FP_LENGTH]); // c0
        limb.reverse();
        p[FP_LENGTH..FP2_LENGTH].copy_from_slice(&limb); // x1 LE

        // y0 <= c1, y1 <= c0
        limb.copy_from_slice(&sum_be[144..192]); // c1
        limb.reverse();
        p[FP2_LENGTH..144].copy_from_slice(&limb); // y0 LE
        limb.copy_from_slice(&sum_be[FP2_LENGTH..144]); // c0
        limb.reverse();
        p[144..192].copy_from_slice(&limb); // y1 LE

        p.copy_from_slice(&result);
    }
}
