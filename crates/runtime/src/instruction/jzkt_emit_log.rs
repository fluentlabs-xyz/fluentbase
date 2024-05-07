use crate::RuntimeContext;
use fluentbase_types::{Address, Bytes, IJournaledTrie, B256};
use rwasm::{core::Trap, Caller};

pub struct JzktEmitLog;

impl JzktEmitLog {
    pub fn fn_fuel_cost(n: u8, len: u64) -> Option<u64> {
        // TODO(dmitry123): "how we can replace it with constants from EVM lib? do we need?"
        375u64
            .checked_add(8u64.checked_mul(len)?)?
            .checked_add(375u64 * n as u64)
    }

    pub fn fn_handler<DB: IJournaledTrie>(
        mut caller: Caller<'_, RuntimeContext<DB>>,
        address20_ptr: u32,
        topics32s_ptr: u32,
        topics32s_len: u32,
        data_ptr: u32,
        data_len: u32,
    ) -> Result<(), Trap> {
        // let fuel_cost = Self::fn_fuel_cost((topics32s_len / 32) as u8, data_len as u64)
        //     .ok_or(ExitCode::OutOfFuel.into_trap())?;
        // match caller.consume_fuel(fuel_cost) {
        //     Ok(_) => {}
        //     Err(err) => match err {
        //         FuelError::OutOfFuel => return Err(ExitCode::OutOfFuel.into_trap()),
        //         _ => {}
        //     },
        // }
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
