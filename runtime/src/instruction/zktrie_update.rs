use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};
use fluentbase_types::ExitCode;

pub struct ZkTrieUpdate;

impl ZkTrieUpdate {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key32_offset: u32,
        flags: u32,
        vals32_offset: u32,
        vals32_len: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key32_offset, 32).to_vec();
        let vals32 = caller
            .read_memory(vals32_offset, vals32_len)
            .chunks(32)
            .map(|v| {
                let mut res = [0u8; 32];
                res.copy_from_slice(v);
                res
            })
            .collect::<Vec<_>>();
        Self::fn_impl(caller.data_mut(), &key, flags, vals32).map_err(|err| err.into_trap())?;
        Ok(())
    }

    pub fn fn_impl<T>(
        context: &mut RuntimeContext<T>,
        key: &[u8],
        value_flags: u32,
        vals: Vec<[u8; 32]>,
    ) -> Result<(), ExitCode> {
        let zktrie = context.zktrie.clone().unwrap();
        zktrie.borrow_mut().update(key, value_flags, &vals)?;
        Ok(())
    }
}
