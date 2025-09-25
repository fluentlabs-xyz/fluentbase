use crate::{
    executor::RuntimeFactoryExecutor, factory::RuntimeFactory, ExecutionResult, RuntimeContext,
    RuntimeExecutor,
};
use fluentbase_types::{Address, BytecodeOrHash, Bytes, B256};
use std::cell::RefCell;

pub struct LocalExecutor;

impl LocalExecutor {
    pub fn new() -> Self {
        LocalExecutor {}
    }
}

thread_local! {
    pub static RUNTIME_FACTORY: RefCell<RuntimeFactory> = RefCell::new(RuntimeFactory::new_v1_preview());
}

impl RuntimeExecutor for LocalExecutor {
    fn execute(
        &mut self,
        bytecode_or_hash: BytecodeOrHash,
        ctx: RuntimeContext,
    ) -> ExecutionResult {
        RUNTIME_FACTORY.with_borrow_mut(|runtime_factory| {
            RuntimeFactoryExecutor::new(runtime_factory).execute(bytecode_or_hash, ctx)
        })
    }

    fn resume(
        &mut self,
        call_id: u32,
        return_data: Vec<u8>,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult {
        RUNTIME_FACTORY.with_borrow_mut(|runtime_factory| {
            RuntimeFactoryExecutor::new(runtime_factory).resume(
                call_id,
                return_data,
                fuel16_ptr,
                fuel_consumed,
                fuel_refunded,
                exit_code,
            )
        })
    }

    fn warmup(&mut self, bytecode: Bytes, hash: B256, address: Address) {
        RUNTIME_FACTORY.with_borrow_mut(|runtime_factory| {
            RuntimeFactoryExecutor::new(runtime_factory).warmup(bytecode, hash, address)
        })
    }

    fn reset_call_id_counter(&mut self) {
        RUNTIME_FACTORY.with_borrow_mut(|runtime_factory| {
            RuntimeFactoryExecutor::new(runtime_factory).reset_call_id_counter()
        })
    }
}
