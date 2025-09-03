use crate::instruction::{
    weierstrass_add::SyscallWeierstrassAddAssign, weierstrass_mul::SyscallWeierstrassMulAssign,
};
use crate::RuntimeContext;
use rwasm::{Store, TrapCode, TypedCaller, Value};
use sp1_curves::weierstrass::bls12_381::Bls12381;

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

        // Each pair is two 64-byte points (G1): [g1_i | scalar_i] where scalar is also encoded as 64 bytes
        // Our helper expects slices of (point, scalar) both 64 bytes.
        let total_len = pairs_len * 128;
        let mut buf = vec![0u8; total_len];
        caller.memory_read(pairs_ptr, &mut buf)?;

        // parse into pairs of (point, scalar)
        let mut pairs: Vec<([u8; 64], [u8; 64])> = Vec::with_capacity(pairs_len);
        for i in 0..pairs_len {
            let start = i * 128;
            let mut p = [0u8; 64];
            let mut s = [0u8; 64];
            p.copy_from_slice(&buf[start..start + 64]);
            s.copy_from_slice(&buf[start + 64..start + 128]);
            pairs.push((p, s));
        }

        let mut out = [0u8; 64];
        Self::fn_impl(&pairs, &mut out);
        caller.memory_write(out_ptr, &out)?;
        Ok(())
    }

    pub fn fn_impl(pairs: &[([u8; 64], [u8; 64])], out: &mut [u8; 64]) {
        let mut acc = [0u8; 64];
        for (p, s64) in pairs.iter() {
            let mut s32 = [0u8; 32];
            s32.copy_from_slice(&s64[..32]);
            let tmp = SyscallWeierstrassMulAssign::<Bls12381>::fn_impl(p, &s32);
            let sum = SyscallWeierstrassAddAssign::<Bls12381>::fn_impl(&acc, &tmp);
            let min = core::cmp::min(acc.len(), sum.len());
            acc[..min].copy_from_slice(&sum[..min]);
        }
        out.copy_from_slice(&acc);
    }
}
