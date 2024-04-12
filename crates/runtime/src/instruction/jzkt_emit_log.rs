use crate::RuntimeContext;
use fluentbase_types::{Address, Bytes, IJournaledTrie, B256};
use rwasm::{core::Trap, Caller};

pub struct JzktEmitLog;

impl JzktEmitLog {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        key32_ptr: u32,
        topics32s_ptr: u32,
        topics32s_len: u32,
        data_ptr: u32,
        data_len: u32,
    ) -> Result<(), Trap> {
        let key = caller.read_memory(key32_ptr, 32)?.to_vec();
        let topics = caller
            .read_memory(topics32s_ptr, topics32s_len)?
            .chunks(32)
            .map(|v| {
                let mut res = B256::ZERO;
                res.copy_from_slice(v);
                res
            })
            .collect::<Vec<_>>();
        let data = caller.read_memory(data_ptr, data_len)?.to_vec();
        Self::fn_impl(caller.data_mut(), &key, &topics, &data);
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        key: &[u8],
        topics: &Vec<B256>,
        data: &[u8],
    ) {
        ctx.jzkt().emit_log(
            Address::from_slice(key),
            topics.clone(),
            Bytes::copy_from_slice(data),
        );
    }
}
