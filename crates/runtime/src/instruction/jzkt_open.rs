use crate::RuntimeContext;
use fluentbase_types::{ExitCode, IJournaledTrie};
use rwasm::{core::Trap, Caller};

pub struct JzktOpen;

impl JzktOpen {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        root32_offset: u32,
    ) -> Result<(), Trap> {
        let root32 = caller.read_memory(root32_offset, 32)?.to_vec();
        Self::fn_impl(caller.data_mut(), &root32).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        _context: &mut RuntimeContext<DB>,
        _root32: &[u8],
    ) -> Result<(), ExitCode> {
        todo!("not implemented yet")
        // let jzkt = context.jzkt.as_mut().expect("jzkt is not set");
        // jzkt.open(root32);
        // Ok(())
    }
}
