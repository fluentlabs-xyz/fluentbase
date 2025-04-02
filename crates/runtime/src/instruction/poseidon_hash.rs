use crate::RuntimeContext;
use fluentbase_rwasm::{Caller, RwasmError};
use fluentbase_types::{ExitCode, B256};
use halo2curves::{bn256::Fr, group::ff::PrimeField};

pub struct SyscallPoseidonHash;

impl SyscallPoseidonHash {
    pub fn fn_handler(mut caller: Caller<'_, RuntimeContext>) -> Result<(), RwasmError> {
        let [fa_offset, fb_offset, fd_offset, output_offset] = caller.stack_pop_n();
        let output = Self::fn_impl(
            &B256::from(caller.memory_read_fixed::<32>(fa_offset.as_usize())?),
            &B256::from(caller.memory_read_fixed::<32>(fb_offset.as_usize())?),
            &B256::from(caller.memory_read_fixed::<32>(fd_offset.as_usize())?),
        )
        .map_err(|err| RwasmError::ExecutionHalted(err.into_i32()))?;
        caller.memory_write(output_offset.as_usize(), output.as_slice())?;
        Ok(())
    }

    pub fn fn_impl(fa: &B256, fb: &B256, fd: &B256) -> Result<B256, ExitCode> {
        use fluentbase_poseidon::hash_with_domain;
        let fr_from_bytes = |fr_data: &B256| -> Result<Fr, ExitCode> {
            let fr = Fr::from_bytes(&fr_data.0);
            if fr.is_none().into() {
                return Err(ExitCode::MalformedBuiltinParams);
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
    // pub fn fn_impl(_fa: &B256, _fb: &B256, _fd: &B256) -> Result<B256, ExitCode> {
    //     unreachable!("poseidon is not supported in `no_std` mode")
    // }
}
