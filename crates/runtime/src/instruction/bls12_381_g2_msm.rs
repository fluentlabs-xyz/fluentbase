use crate::RuntimeContext;
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
        let pairs_len = params[1].i32().unwrap() as usize; // number of pairs
        let out_ptr = params[2].i32().unwrap() as usize;

        // Each pair: 192-byte point + 32-byte scalar
        let total_len = pairs_len * (192 + 32);
        let mut buf = vec![0u8; total_len];
        caller.memory_read(pairs_ptr, &mut buf)?;

        // parse into pairs of (point192, scalar32)
        let mut pairs: Vec<([u8; 192], [u8; 32])> = Vec::with_capacity(pairs_len);
        for i in 0..pairs_len {
            let start = i * (192 + 32);
            let mut p = [0u8; 192];
            let mut s = [0u8; 32];
            p.copy_from_slice(&buf[start..start + 192]);
            s.copy_from_slice(&buf[start + 192..start + 224]);
            pairs.push((p, s));
        }

        let mut out = [0u8; 192];
        Self::fn_impl(&pairs, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(pairs: &[([u8; 192], [u8; 32])], out: &mut [u8; 192]) {
        let mut acc = G2Projective::identity();

        let mut limb = [0u8; 48];
        for (p, s32) in pairs.iter() {
            // Convert point to BE uncompressed (swap c0/c1 like in add)
            let mut be = [0u8; 192];
            // x: c0 <= x1, c1 <= x0
            limb.copy_from_slice(&p[48..96]);
            limb.reverse();
            be[0..48].copy_from_slice(&limb);
            limb.copy_from_slice(&p[0..48]);
            limb.reverse();
            be[48..96].copy_from_slice(&limb);
            // y: c0 <= y1, c1 <= y0
            limb.copy_from_slice(&p[144..192]);
            limb.reverse();
            be[96..144].copy_from_slice(&limb);
            limb.copy_from_slice(&p[96..144]);
            limb.reverse();
            be[144..192].copy_from_slice(&limb);

            let a_aff_opt = G2Affine::from_uncompressed(&be);
            if a_aff_opt.is_none().unwrap_u8() == 1 {
                out.fill(0);
                return;
            }
            let a = G2Projective::from(a_aff_opt.unwrap());
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

        // Serialize acc to BE uncompressed and map back to LE limbs
        let sum_aff = G2Affine::from(acc);
        let be = sum_aff.to_uncompressed();
        // x0 <= c1, x1 <= c0
        limb.copy_from_slice(&be[48..96]);
        limb.reverse();
        out[0..48].copy_from_slice(&limb);
        limb.copy_from_slice(&be[0..48]);
        limb.reverse();
        out[48..96].copy_from_slice(&limb);
        // y0 <= c1, y1 <= c0
        limb.copy_from_slice(&be[144..192]);
        limb.reverse();
        out[96..144].copy_from_slice(&limb);
        limb.copy_from_slice(&be[96..144]);
        limb.reverse();
        out[144..192].copy_from_slice(&limb);
    }
}
