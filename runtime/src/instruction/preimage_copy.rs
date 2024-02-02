use crate::RuntimeContext;
use fluentbase_types::ExitCode;
use rwasm::{common::Trap, Caller};

pub struct PreimageCopy;

impl PreimageCopy {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        hash32_offset: u32,
        output_offset: u32,
        output_len: u32,
    ) -> Result<(), Trap> {
        let hash = caller.read_memory(hash32_offset, 32).to_vec();
        let preimage =
            Self::fn_impl(caller.data(), &hash, output_len).map_err(|err| err.into_trap())?;
        caller.write_memory(output_offset, &preimage);
        Ok(())
    }

    pub fn fn_impl<T>(
        ctx: &RuntimeContext<T>,
        hash: &[u8],
        output_len: u32,
    ) -> Result<Vec<u8>, ExitCode> {
        let preimage_db = ctx
            .preimage_db
            .as_ref()
            .ok_or(ExitCode::PreimageUnavailable)?;
        let preimage_size = preimage_db.borrow().preimage_size(hash);
        if preimage_size > output_len {
            return Err(ExitCode::OutputOverflow);
        }
        let mut preimage = vec![0; preimage_size as usize];
        preimage_db.borrow().copy_preimage(hash, &mut preimage);
        Ok(preimage)
    }
}
