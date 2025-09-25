use crate::{
    factory::RuntimeFactory, local_executor::LocalExecutor,
    syscall_handler::runtime_syscall_handler, ExecutionResult, Runtime, RuntimeContext,
};
use fluentbase_types::{Address, BytecodeOrHash, Bytes, B256};
use rwasm::FuelConfig;

pub trait RuntimeExecutor {
    fn execute(&mut self, bytecode_or_hash: BytecodeOrHash, ctx: RuntimeContext)
        -> ExecutionResult;

    fn resume(
        &mut self,
        call_id: u32,
        return_data: Vec<u8>,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult;

    fn warmup(&mut self, bytecode: Bytes, hash: B256, address: Address);

    /// Resets the per-transaction call identifier counter and clears recoverable runtimes.
    ///
    /// Intended to be invoked at the beginning of a new transaction.
    fn reset_call_id_counter(&mut self);
}

pub struct RuntimeFactoryExecutor<'a> {
    runtime_factory: &'a mut RuntimeFactory,
}

impl<'a> RuntimeFactoryExecutor<'a> {
    pub fn new(runtime_factory: &'a mut RuntimeFactory) -> Self {
        Self { runtime_factory }
    }
}

impl<'a> RuntimeExecutor for RuntimeFactoryExecutor<'a> {
    fn execute(
        &mut self,
        bytecode_or_hash: BytecodeOrHash,
        ctx: RuntimeContext,
    ) -> ExecutionResult {
        // Get code hash before moving `bytecode_or_hash` (we store it inside runtime)
        let code_hash = bytecode_or_hash.code_hash();

        // If we have a cached module, then use it, otherwise create a new one and cache
        let strategy = self.runtime_factory.get_module_or_init(bytecode_or_hash);

        // If there is no cached store, then construct a new one (slow)
        let fuel_config = FuelConfig::default().with_fuel_limit(ctx.fuel_limit);
        let store = strategy.create_store(
            self.runtime_factory.import_linker.clone(),
            ctx,
            runtime_syscall_handler,
            fuel_config,
        );

        // Execute an app
        let mut runtime = Runtime::new(strategy, store, code_hash);
        let runtime_result = runtime.execute();
        let result = self
            .runtime_factory
            .try_remember_runtime(runtime_result, runtime);
        result
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
        let mut runtime = self.runtime_factory.recover_runtime(call_id);
        let runtime_result = runtime.resume(
            return_data,
            fuel16_ptr,
            fuel_consumed,
            fuel_refunded,
            exit_code,
        );
        let result = self
            .runtime_factory
            .try_remember_runtime(runtime_result, runtime);
        result
    }

    fn warmup(&mut self, bytecode: Bytes, hash: B256, address: Address) {
        self.runtime_factory
            .get_module_or_init(BytecodeOrHash::Bytecode {
                bytecode,
                hash,
                address,
            });
    }

    fn reset_call_id_counter(&mut self) {
        self.runtime_factory.reset_call_id_counter();
    }
}

pub fn default_runtime_executor() -> impl RuntimeExecutor {
    LocalExecutor {}
}
