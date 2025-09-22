use crate::{
    syscall_handler::bls12381::{
        bls12381_consts::{G2_UNCOMPRESSED_LENGTH, SCALAR_LENGTH},
        bls12381_helpers::{
            g2_be_uncompressed_to_le_limbs, g2_le_limbs_to_be_uncompressed, parse_affine_g2,
        },
    },
    RuntimeContext,
};
use blstrs::{G2Affine, G2Projective, Scalar};
use group::Group;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallBls12381G2Msm;

impl SyscallBls12381G2Msm {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let pairs_ptr = params[0].i32().unwrap() as usize;
        // number of pairs
        let pairs_len = params[1].i32().unwrap() as usize;
        let out_ptr = params[2].i32().unwrap() as usize;

        // Each pair: 192-byte point + 32-byte scalar
        let total_len = pairs_len * (G2_UNCOMPRESSED_LENGTH + SCALAR_LENGTH);
        let mut buf = vec![0u8; total_len];
        caller.memory_read(pairs_ptr, &mut buf)?;

        // parse into pairs of (point192, scalar32)
        let mut pairs: Vec<([u8; G2_UNCOMPRESSED_LENGTH], [u8; SCALAR_LENGTH])> =
            Vec::with_capacity(pairs_len);
        for i in 0..pairs_len {
            let start = i * (G2_UNCOMPRESSED_LENGTH + SCALAR_LENGTH);
            let mut p = [0u8; G2_UNCOMPRESSED_LENGTH];
            let mut s = [0u8; SCALAR_LENGTH];
            p.copy_from_slice(&buf[start..start + G2_UNCOMPRESSED_LENGTH]);
            s.copy_from_slice(
                &buf[start + G2_UNCOMPRESSED_LENGTH
                    ..start + G2_UNCOMPRESSED_LENGTH + SCALAR_LENGTH],
            );
            pairs.push((p, s));
        }

        let mut out = [0u8; G2_UNCOMPRESSED_LENGTH];
        Self::fn_impl(&pairs, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(
        pairs: &[([u8; G2_UNCOMPRESSED_LENGTH], [u8; SCALAR_LENGTH])],
        out: &mut [u8; G2_UNCOMPRESSED_LENGTH],
    ) {
        let mut acc = G2Projective::identity();

        for (p, s32) in pairs.iter() {
            // Convert point to BE uncompressed using shared helper
            let be = g2_le_limbs_to_be_uncompressed(p);

            let a_aff = parse_affine_g2(&be);
            let a = G2Projective::from(a_aff);
            let scalar_opt = Scalar::from_bytes_le(s32);
            if scalar_opt.is_none().unwrap_u8() == 1 {
                continue;
            }
            let scalar = scalar_opt.unwrap();
            acc += &(a * &scalar);
        }

        // If identity, return zeroed limbs (runtime convention)
        if acc.is_identity().unwrap_u8() == 1 {
            out.fill(0);
            return;
        }

        // Serialize acc to BE uncompressed and convert back to LE limbs
        let sum_aff = G2Affine::from(acc);
        out.copy_from_slice(&g2_be_uncompressed_to_le_limbs(&sum_aff.to_uncompressed()));
    }
}
