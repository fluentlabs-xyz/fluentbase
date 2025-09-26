use super::bls12381_consts::FP_PAD_BY;
use crate::{
    syscall_handler::bls12381::bls12381_consts::{
        FP_LENGTH, G1_UNCOMPRESSED_LENGTH, PADDED_FP_LENGTH,
    },
    RuntimeContext,
};
use blst::{
    blst_fp, blst_fp_from_bendian, blst_map_to_g1, blst_p1, blst_p1_affine,
    blst_p1_affine_serialize, blst_p1_to_affine,
};
use rwasm::{Store, TrapCode, Value};

pub struct SyscallBls12381MapFpToG1;

impl SyscallBls12381MapFpToG1 {
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let out_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; PADDED_FP_LENGTH];
        caller.memory_read(p_ptr, &mut p)?;

        let mut out = [0u8; G1_UNCOMPRESSED_LENGTH];
        Self::fn_impl(&p, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(p: &[u8; 64], out: &mut [u8; G1_UNCOMPRESSED_LENGTH]) {
        // Input is 64B BE EIP-2537 padded Fp. Strip the first 16 zero bytes to get 48B BE field element.
        let mut fp_be = [0u8; FP_LENGTH];
        fp_be.copy_from_slice(&p[FP_PAD_BY..PADDED_FP_LENGTH]);

        // Convert BE bytes into blst_fp
        let mut fp = blst_fp { l: [0u64; 6] };
        unsafe { blst_fp_from_bendian(&mut fp as *mut blst_fp, fp_be.as_ptr()) };

        // Map to curve using blst
        let mut p1 = blst_p1::default();
        unsafe {
            blst_map_to_g1(
                &mut p1 as *mut blst_p1,
                &fp as *const blst_fp,
                core::ptr::null(),
            )
        };

        // Convert to affine
        let mut p1_aff = blst_p1_affine::default();
        unsafe { blst_p1_to_affine(&mut p1_aff as *mut blst_p1_affine, &p1 as *const blst_p1) };

        // Serialize as uncompressed 96B (x||y)
        unsafe { blst_p1_affine_serialize(out.as_mut_ptr(), &p1_aff as *const blst_p1_affine) };
    }
}
