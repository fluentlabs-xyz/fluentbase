use super::ecc_config::MapConfig;
/// Generic mapping handler for Weierstrass curves, specifically for
/// - Fp(size: 64) to G1(size: 96)
/// - Fp2(size: 128) to G2(size: 192)
///
/// This module provides generic handlers for mapping field elements to curve points
/// using the BLST library for high-performance operations.
///
use crate::RuntimeContext;
use blst::{
    blst_fp, blst_fp2, blst_fp_from_bendian, blst_map_to_g1, blst_map_to_g2, blst_p1,
    blst_p1_affine, blst_p1_affine_serialize, blst_p1_to_affine, blst_p2, blst_p2_affine,
    blst_p2_affine_serialize, blst_p2_to_affine,
};
use fluentbase_types::{
    FP_PAD_BY, FP_SIZE, G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE, PADDED_FP2_SIZE, PADDED_FP_SIZE,
};
use rwasm::{Store, TrapCode, Value};
use sp1_curves::CurveType;
use std::marker::PhantomData;

pub struct SyscallEccMapping<C: MapConfig> {
    _phantom: PhantomData<C>,
}

impl<C: MapConfig> SyscallEccMapping<C> {
    pub fn fn_handler(
        caller: &mut impl Store<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let out_ptr = params[1].i32().unwrap() as usize;

        // Use config constants for input/output sizes
        let mut input_buffer = vec![0u8; C::INPUT_SIZE];
        caller.memory_read(p_ptr, &mut input_buffer)?;

        let output = Self::fn_impl(&input_buffer);
        caller.memory_write(out_ptr, &output)?;

        Ok(())
    }

    pub fn fn_impl(input: &[u8]) -> Vec<u8> {
        if input.len() >= C::INPUT_SIZE {
            let input_array: Vec<u8> = input[..C::INPUT_SIZE].to_vec();

            match C::CURVE_TYPE {
                CurveType::Bls12381 => {
                    if C::INPUT_SIZE == PADDED_FP_SIZE && C::OUTPUT_SIZE == G1_UNCOMPRESSED_SIZE {
                        // Fp to G1 mapping
                        let fp_input: [u8; PADDED_FP_SIZE] =
                            input_array.try_into().unwrap_or([0u8; PADDED_FP_SIZE]);
                        let mut output_array: [u8; G1_UNCOMPRESSED_SIZE] =
                            [0u8; G1_UNCOMPRESSED_SIZE];
                        Self::fn_impl_fp_to_g1_internal(&fp_input, &mut output_array);
                        output_array.to_vec()
                    } else if C::INPUT_SIZE == PADDED_FP2_SIZE
                        && C::OUTPUT_SIZE == G2_UNCOMPRESSED_SIZE
                    {
                        // Fp2 to G2 mapping
                        let fp2_input: [u8; PADDED_FP2_SIZE] =
                            input_array.try_into().unwrap_or([0u8; PADDED_FP2_SIZE]);
                        let mut output_array: [u8; G2_UNCOMPRESSED_SIZE] =
                            [0u8; G2_UNCOMPRESSED_SIZE];
                        Self::fn_impl_fp2_to_g2_internal(&fp2_input, &mut output_array);
                        output_array.to_vec()
                    } else {
                        vec![0u8; C::OUTPUT_SIZE]
                    }
                }
                _ => vec![0u8; C::OUTPUT_SIZE], // Unsupported curve type
            }
        } else {
            vec![0u8; C::OUTPUT_SIZE]
        }
    }

    /// Generic implementation that returns the result for context_wrapper compatibility
    pub fn fn_impl_fp_to_g1(p: &[u8; PADDED_FP_SIZE]) -> [u8; G1_UNCOMPRESSED_SIZE] {
        let mut output = [0u8; G1_UNCOMPRESSED_SIZE];
        Self::fn_impl_fp_to_g1_internal(p, &mut output);
        output
    }

    /// Generic implementation that returns the result for context_wrapper compatibility
    pub fn fn_impl_fp2_to_g2(p: &[u8; PADDED_FP2_SIZE]) -> [u8; G2_UNCOMPRESSED_SIZE] {
        let mut output = [0u8; G2_UNCOMPRESSED_SIZE];
        Self::fn_impl_fp2_to_g2_internal(p, &mut output);
        output
    }

    /// Internal Fp to G1 mapping implementation
    fn fn_impl_fp_to_g1_internal(p: &[u8; PADDED_FP_SIZE], out: &mut [u8; G1_UNCOMPRESSED_SIZE]) {
        // Input is 64B BE EIP-2537 padded Fp. Strip the first 16 zero bytes to get 48B BE field element.
        let mut fp_be = [0u8; FP_SIZE];
        fp_be.copy_from_slice(&p[FP_PAD_BY..PADDED_FP_SIZE]);

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

    /// Internal Fp2 to G2 mapping implementation
    fn fn_impl_fp2_to_g2_internal(p: &[u8; PADDED_FP2_SIZE], out: &mut [u8; G2_UNCOMPRESSED_SIZE]) {
        // Interpret input as two 64-byte BE padded limbs (c0||c1). Extract 48-byte BE (skip 16 leading zeros per limb).
        let mut c0_be48 = [0u8; FP_SIZE];
        let mut c1_be48 = [0u8; FP_SIZE];
        c0_be48.copy_from_slice(&p[FP_PAD_BY..PADDED_FP_SIZE]);
        c1_be48.copy_from_slice(&p[PADDED_FP_SIZE + FP_PAD_BY..PADDED_FP2_SIZE]);

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
