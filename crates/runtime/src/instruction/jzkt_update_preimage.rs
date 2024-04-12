use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct JzktUpdatePreimage;

impl JzktUpdatePreimage {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        key32_ptr: u32,
        field: u32,
        preimage_ptr: u32,
        preimage_len: u32,
    ) -> Result<i32, Trap> {
        let key = caller.read_memory(key32_ptr, 32)?.to_vec();
        let preimage = caller.read_memory(preimage_ptr, preimage_len)?.to_vec();
        let res = Self::fn_impl(caller.data_mut(), &key, field, &preimage)
            .map_err(|err| err.into_trap())?;
        Ok(res as i32)
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        key: &[u8],
        field: u32,
        preimage: &[u8],
    ) -> Result<bool, ExitCode> {
        let res = ctx
            .jzkt()
            .update_preimage(key.try_into().unwrap(), field, preimage);
        Ok(res)
    }
}
