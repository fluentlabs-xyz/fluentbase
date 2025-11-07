use crate::{
    executor::{ExecutionResult, RuntimeFactoryExecutor},
    RuntimeContext, RuntimeExecutor,
};
use fluentbase_types::{import_linker_v1_preview, Address, BytecodeOrHash, B256};
use rwasm::{RwasmModule, TrapCode};
use std::cell::RefCell;

pub struct LocalExecutor;

impl LocalExecutor {
    pub fn new() -> Self {
        LocalExecutor {}
    }
}

thread_local! {
    pub static LOCAL_RUNTIME_EXECUTOR: RefCell<RuntimeFactoryExecutor> = RefCell::new(RuntimeFactoryExecutor::new(import_linker_v1_preview()));
}

impl RuntimeExecutor for LocalExecutor {
    fn execute(
        &mut self,
        bytecode_or_hash: BytecodeOrHash,
        ctx: RuntimeContext,
    ) -> ExecutionResult {
        LOCAL_RUNTIME_EXECUTOR
            .with_borrow_mut(|runtime_executor| runtime_executor.execute(bytecode_or_hash, ctx))
    }

    fn resume(
        &mut self,
        call_id: u32,
        return_data: &[u8],
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult {
        LOCAL_RUNTIME_EXECUTOR.with_borrow_mut(|runtime_executor| {
            runtime_executor.resume(
                call_id,
                return_data,
                fuel16_ptr,
                fuel_consumed,
                fuel_refunded,
                exit_code,
            )
        })
    }

    fn forget_runtime(&mut self, call_id: u32) {
        LOCAL_RUNTIME_EXECUTOR
            .with_borrow_mut(|runtime_executor| runtime_executor.forget_runtime(call_id))
    }

    fn warmup(&mut self, bytecode: RwasmModule, hash: B256, address: Address) {
        LOCAL_RUNTIME_EXECUTOR
            .with_borrow_mut(|runtime_executor| runtime_executor.warmup(bytecode, hash, address))
    }

    #[cfg(feature = "wasmtime")]
    fn warmup_wasmtime(
        &mut self,
        rwasm_module: RwasmModule,
        wasmtime_module: wasmtime::Module,
        code_hash: B256,
    ) {
        LOCAL_RUNTIME_EXECUTOR.with_borrow_mut(|runtime_executor| {
            runtime_executor.warmup_wasmtime(rwasm_module, wasmtime_module, code_hash)
        })
    }

    fn reset_call_id_counter(&mut self) {
        LOCAL_RUNTIME_EXECUTOR
            .with_borrow_mut(|runtime_executor| runtime_executor.reset_call_id_counter())
    }

    fn memory_read(
        &mut self,
        call_id: u32,
        offset: usize,
        buffer: &mut [u8],
    ) -> Result<(), TrapCode> {
        LOCAL_RUNTIME_EXECUTOR.with_borrow_mut(|runtime_executor| {
            runtime_executor.memory_read(call_id, offset, buffer)
        })
    }
}
