use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};
use fluentbase_types::{ExitCode, F254};
use halo2curves::{bn256::Fr, group::ff::PrimeField};

pub struct SyscallPoseidonHash;

impl SyscallPoseidonHash {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [fa_offset, fb_offset, fd_offset, output_offset] = caller.stack_pop_n();
        let output = Self::fn_impl(
            &F254::from(caller.memory_read_fixed::<32>(fa_offset.as_usize())?),
            &F254::from(caller.memory_read_fixed::<32>(fb_offset.as_usize())?),
            &F254::from(caller.memory_read_fixed::<32>(fd_offset.as_usize())?),
        )
        .map_err(|err| RwasmError::ExecutionHalted(err.into_i32()))?;
        caller.memory_write(output_offset.as_usize(), output.as_slice())?;
        Ok(())
    }

    pub fn fn_impl(fa: &F254, fb: &F254, fd: &F254) -> Result<F254, ExitCode> {
        use fluentbase_poseidon::hash_with_domain;
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

    // #[cfg(not(feature = "std"))]
    // pub fn fn_impl(_fa: &F254, _fb: &F254, _fd: &F254) -> Result<F254, ExitCode> {
    //     unreachable!("poseidon is not supported in `no_std` mode")
    // }
}
