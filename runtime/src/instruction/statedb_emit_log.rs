use crate::{types::B256, RuntimeContext};
use fluentbase_rwasm::{common::Trap, Caller};

pub struct StateDbEmitLog;

impl StateDbEmitLog {
    pub fn fn_handler<T>(
        mut caller: Caller<'_, RuntimeContext<T>>,
        topics32_offset: u32,
        topics32_length: u32,
        data_offset: u32,
        data_len: u32,
    ) -> Result<(), Trap> {
        let topics = caller
            .read_memory(topics32_offset, topics32_length)
            .to_vec()
            .chunks(32)
            .map(|val| {
                let mut result = [0u8; 32];
                result[0..val.len()].copy_from_slice(val);
                B256::from(result)
            })
            .collect::<Vec<_>>();
        let data = caller.read_memory(data_offset, data_len).to_vec();
        Self::fn_impl(caller.data_mut(), &topics, &data);
        Ok(())
    }

    pub fn fn_impl<T>(context: &mut RuntimeContext<T>, topics: &Vec<B256>, data: &Vec<u8>) {
        let account_db = context.account_db.clone().unwrap();
        account_db
            .borrow_mut()
            .emit_log(&context.address, &topics, data.clone().into());
    }
}
