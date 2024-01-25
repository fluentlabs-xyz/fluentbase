use crate::RuntimeContext;
use fluentbase_rwasm::{common::Trap, Caller};

pub struct ZkTrieSetCode;

impl ZkTrieSetCode {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        key20_offset: u32,
        code_offset: u32,
        code_len: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key20_offset, 20).to_vec();
        let code = caller.read_memory(code_offset, code_len).to_vec();
        Self::fn_impl(caller.data_mut(), &key, &code);
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, key: &[u8], code: &[u8]) {
        let zktrie = context.zktrie.clone().unwrap();
        zktrie.borrow_mut().set_code(key, code);
    }
}
