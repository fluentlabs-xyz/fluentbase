use crate::RuntimeContext;
use fluentbase_types::{Address, Bytes, IJournaledTrie, B256};
use rwasm::{core::Trap, Caller};

pub struct JzktEmitLog;

impl JzktEmitLog {
    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        address20_ptr: u32,
        topics32s_ptr: u32,
        topics32s_len: u32,
        data_ptr: u32,
        data_len: u32,
    ) -> Result<(), Trap> {
        let address = Address::from_slice(caller.read_memory(address20_ptr, 20)?);
        let topics = caller
            .read_memory(topics32s_ptr, topics32s_len)?
            .chunks(32)
            .map(|v| {
                let mut res = B256::ZERO;
                res.copy_from_slice(v);
                res
            })
            .collect::<Vec<_>>();
        let data = Bytes::copy_from_slice(caller.read_memory(data_ptr, data_len)?);
        Self::fn_impl(caller.data_mut(), address, topics, data);
        Ok(())
    }

    pub fn fn_impl<DB: IJournaledTrie>(
        ctx: &mut RuntimeContext<DB>,
        address: Address,
        topics: Vec<B256>,
        data: Bytes,
    ) {
        ctx.jzkt().emit_log(address, topics.clone(), data);
    }
}
