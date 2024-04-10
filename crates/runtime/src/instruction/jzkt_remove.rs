use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct JzktRemove;

impl JzktRemove {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        key32_offset: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key32_offset, 32)?.to_vec();
        Self::fn_impl(caller.data_mut(), &key).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        context: &mut RuntimeContext<DB>,
        key: &[u8],
    ) -> Result<(), ExitCode> {
        let jzkt = context.jzkt.as_mut().expect("jzkt is not set");
        jzkt.remove(key.try_into().unwrap());
        Ok(())
    }
}
