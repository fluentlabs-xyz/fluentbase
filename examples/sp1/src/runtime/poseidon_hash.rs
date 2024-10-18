use crate::RuntimeContext;
use fluentbase_poseidon::hash_with_domain;
use fluentbase_types::{ExitCode, F254};
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use rwasm::{core::Trap, Caller};

pub struct SyscallPoseidonHash;

impl SyscallPoseidonHash {
    pub fn fn_handler(
        mut caller: Caller<'_, RuntimeContext>,
        fa_offset: u32,
        fb_offset: u32,
        fd_offset: u32,
        output_offset: u32,
    ) -> Result<(), Trap> {
        let output = Self::fn_impl(
            &F254::from_slice(caller.read_memory(fa_offset, 32)?),
            &F254::from_slice(caller.read_memory(fb_offset, 32)?),
            &F254::from_slice(caller.read_memory(fd_offset, 32)?),
        )
        .map_err(|err| err.into_trap())?;
        caller.write_memory(output_offset, output.as_slice())?;
        Ok(())
    }

    pub fn fn_impl(fa: &F254, fb: &F254, fd: &F254) -> Result<F254, ExitCode> {
        let fr_from_bytes = |fr_data: &F254| -> Result<Fr, ExitCode> {
            let fr = Fr::from_bytes(&fr_data.0);
            if fr.is_none().into() {
                return Err(ExitCode::PoseidonError);
            }
            Ok(fr.unwrap())
        };
        let fa = fr_from_bytes(fa)?;
        let fb = fr_from_bytes(fb)?;
        let fd = fr_from_bytes(fd)?;
        let h2 = hash_with_domain(&[fa, fb], &fd);
        Ok(h2.to_repr().into())
    }
}
