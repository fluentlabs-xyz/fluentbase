use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::ExitCode;

pub struct ZkTrieOpen;

impl ZkTrieOpen {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        root32_offset: u32,
    ) -> Result<(), Trap> {
        let root32 = caller.read_memory(root32_offset, 32).to_vec();
        Self::fn_impl(caller.data_mut(), &root32).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, root32: &[u8]) -> Result<(), ExitCode> {
        let zktrie = context.zktrie.clone().unwrap();
        zktrie.borrow_mut().open(root32);
        Ok(())
    }
}
