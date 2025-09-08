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
        let pairs_len = params[1].i32().unwrap() as usize; // number of pairs
        let out_ptr = params[2].i32().unwrap() as usize;

        // Each pair is (point||scalar): 96-byte uncompressed G1 (BE) + 32-byte scalar (LE)
        let total_len = pairs_len * (96 + 32);
        let mut buf = vec![0u8; total_len];
        caller.memory_read(pairs_ptr, &mut buf)?;

        // parse into pairs of (point, scalar)
        let mut pairs: Vec<([u8; 96], [u8; 32])> = Vec::with_capacity(pairs_len);
        for i in 0..pairs_len {
            let start = i * (96 + 32);
            let mut p = [0u8; 96];
            let mut s = [0u8; 32];
            p.copy_from_slice(&buf[start..start + 96]);
            s.copy_from_slice(&buf[start + 96..start + 96 + 32]);
            pairs.push((p, s));
        }

        let mut out = [0u8; 96];
        Self::fn_impl(&pairs, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(pairs: &[([u8; 96], [u8; 32])], out: &mut [u8; 96]) {
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
        let result_aff = G1Affine::from(acc);
        *out = result_aff.to_uncompressed();
    }
}
