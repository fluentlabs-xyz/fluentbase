use crate::instruction::bls12_381_consts::{FP_LENGTH, G2_UNCOMPRESSED_LENGTH};
use crate::RuntimeContext;
use blst::{
    blst_fp, blst_fp2, blst_fp_from_bendian, blst_map_to_g2, blst_p2, blst_p2_affine,
    blst_p2_affine_serialize, blst_p2_to_affine,
};
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallBls12381MapFp2ToG2;

impl SyscallBls12381MapFp2ToG2 {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let out_ptr = params[1].i32().unwrap() as usize;

        // Read 128-byte padded Fp2: 64B c0 || 64B c1
        let mut p = [0u8; 128];
        caller.memory_read(p_ptr, &mut p)?;

        let mut out = [0u8; G2_UNCOMPRESSED_LENGTH];
        Self::fn_impl(&p, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(p: &[u8; 128], out: &mut [u8; G2_UNCOMPRESSED_LENGTH]) {
        // Interpret input as two 64-byte BE padded limbs (c0||c1). Extract 48-byte BE (skip 16 leading zeros per limb).
        let mut c0_be48 = [0u8; FP_LENGTH];
        let mut c1_be48 = [0u8; FP_LENGTH];
        c0_be48.copy_from_slice(&p[16..64]);
        c1_be48.copy_from_slice(&p[64 + 16..64 + 64]);

        // Convert to blst_fp elements
        let mut u0 = blst_fp { l: [0u64; 6] };
        let mut u1 = blst_fp { l: [0u64; 6] };
        unsafe {
            blst_fp_from_bendian(&mut u0 as *mut blst_fp, c0_be48.as_ptr());
            blst_fp_from_bendian(&mut u1 as *mut blst_fp, c1_be48.as_ptr());
        }

        // Construct Fp2
        let fp2 = blst_fp2 { fp: [u0, u1] };
        // Map to G2 and convert to affine
        let mut p2 = blst_p2::default();
        unsafe {
            blst_map_to_g2(
                &mut p2 as *mut blst_p2,
                &fp2 as *const blst_fp2,
                core::ptr::null(),
            )
        };
        let mut p2_aff = blst_p2_affine::default();
        unsafe { blst_p2_to_affine(&mut p2_aff as *mut blst_p2_affine, &p2 as *const blst_p2) };

        // Serialize affine (192 bytes: x0||x1||y0||y1; each limb 48B BE)
        unsafe { blst_p2_affine_serialize(out.as_mut_ptr(), &p2_aff as *const blst_p2_affine) };
    }
}
