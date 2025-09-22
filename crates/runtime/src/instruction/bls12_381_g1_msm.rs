use crate::instruction::bls12_381_consts::{G1_UNCOMPRESSED_LENGTH, SCALAR_LENGTH};
use crate::instruction::bls12_381_helpers::parse_affine_g1;
use crate::RuntimeContext;
use blstrs::{G1Affine, G1Projective, Scalar};
use group::Group;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallBls12381G1Msm;

impl SyscallBls12381G1Msm {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let pairs_ptr = params[0].i32().unwrap() as usize;
        let pairs_len = params[1].i32().unwrap() as usize;
        let out_ptr = params[2].i32().unwrap() as usize;

        // Each pair is (point||scalar): 96-byte uncompressed G1 (BE) + 32-byte scalar (LE)
        let total_len = pairs_len * (G1_UNCOMPRESSED_LENGTH + SCALAR_LENGTH);
        let mut buf = vec![0u8; total_len];
        caller.memory_read(pairs_ptr, &mut buf)?;

        let mut pairs: Vec<([u8; G1_UNCOMPRESSED_LENGTH], [u8; SCALAR_LENGTH])> =
            Vec::with_capacity(pairs_len);
        for i in 0..pairs_len {
            let start = i * (G1_UNCOMPRESSED_LENGTH + SCALAR_LENGTH);
            let mut p = [0u8; G1_UNCOMPRESSED_LENGTH];
            let mut s = [0u8; SCALAR_LENGTH];
            p.copy_from_slice(&buf[start..start + G1_UNCOMPRESSED_LENGTH]);
            s.copy_from_slice(
                &buf[start + G1_UNCOMPRESSED_LENGTH
                    ..start + G1_UNCOMPRESSED_LENGTH + SCALAR_LENGTH],
            );
            pairs.push((p, s));
        }

        let mut out = [0u8; G1_UNCOMPRESSED_LENGTH];
        Self::fn_impl(&pairs, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(
        pairs: &[([u8; G1_UNCOMPRESSED_LENGTH], [u8; SCALAR_LENGTH])],
        out: &mut [u8; G1_UNCOMPRESSED_LENGTH],
    ) {
        let mut acc = G1Projective::identity();
        for (p, s_le) in pairs.iter() {
            let p_aff = parse_affine_g1(p);
            let s_opt = Scalar::from_bytes_le(s_le);
            if s_opt.is_none().into() {
                // Treat invalid scalar as zero contribution
                continue;
            }
            let term = G1Projective::from(p_aff) * s_opt.unwrap();
            acc += term;
        }
        out.copy_from_slice(&G1Affine::from(acc).to_uncompressed());
    }
}
