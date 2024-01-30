use crate::RuntimeContext;
use fluentbase_poseidon::Hashable;
use fluentbase_types::ExitCode;
use halo2curves::{bn256::Fr, group::ff::PrimeField};
use rwasm::{common::Trap, Caller};

pub struct CryptoPoseidon2;

impl CryptoPoseidon2 {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        fa_offset: u32,
        fb_offset: u32,
        fd_offset: u32,
        output_offset: u32,
    ) -> Result<(), Trap> {
        let output = Self::fn_impl(
            caller.read_memory(fa_offset, 32),
            caller.read_memory(fb_offset, 32),
            caller.read_memory(fd_offset, 32),
        )?;
        caller.write_memory(output_offset, &output);
        Ok(())
    }

    pub fn fn_impl(fa: &[u8], fb: &[u8], fd: &[u8]) -> Result<[u8; 32], Trap> {
        let fr_from_bytes = |fr_data: &[u8]| -> Result<Fr, Trap> {
            let fr_data: [u8; 32] = fr_data
                .try_into()
                .map_err(|_| <ExitCode as Into<Trap>>::into(ExitCode::PoseidonError))?;
            let fr = Fr::from_bytes(&fr_data);
            if fr.is_none().into() {
                return Err(<ExitCode as Into<Trap>>::into(ExitCode::PoseidonError));
            }
            Ok(fr.unwrap())
        };
        let fa = fr_from_bytes(fa)?;
        let fb = fr_from_bytes(fb)?;
        let fd = fr_from_bytes(fd)?;
        let hasher = Fr::hasher();
        let h2 = hasher.hash([fa, fb], fd);
        Ok(h2.to_repr())
    }
}
