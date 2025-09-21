use crate::{
    context::RuntimeContext, factory::CACHING_RUNTIME_FACTORY,
    instruction::exec::InterruptionHolder, syscall_handler::runtime_syscall_handler,
};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    Address, BytecodeOrHash, Bytes, ExitCode, B256, STATE_DEPLOY, STATE_MAIN,
};
use rwasm::{Store, Strategy, TrapCode, TypedStore, Value};
use std::{fmt::Debug, mem::take, sync::Arc};

#[derive(Default, Clone, Debug)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub fuel_consumed: u64,
    pub fuel_refunded: i64,
    /// A return data from nested call
    pub return_data: Vec<u8>,
    pub output: Vec<u8>,
    /// Was call interrupted by a system call
    pub interrupted: bool,
}

pub struct Runtime {
    pub strategy: Arc<Strategy>,
    pub store: TypedStore<RuntimeContext>,
    pub code_hash: B256,
}

impl Runtime {
    #[deprecated(note = "use `new` method instead")]
    pub fn run_with_context(
        bytecode_or_hash: BytecodeOrHash,
        runtime_context: RuntimeContext,
    ) -> ExecutionResult {
        Self::new(bytecode_or_hash, runtime_context).execute()
    }

    #[tracing::instrument(level = "info", skip_all)]
    pub fn new(bytecode_or_hash: BytecodeOrHash, runtime_context: RuntimeContext) -> Self {
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            let code_hash = bytecode_or_hash.code_hash();

            // If we have a cached module, then use it, otherwise create a new one and cache
            let cached_module = caching_runtime.get_module_or_init(bytecode_or_hash);
            let (strategy, mut store) = cached_module.acquire_shared();

            // If there is cached store then reuse it, but rewrite the context data
            if let Some(store) = store.as_mut() {
                match store {
                    TypedStore::Rwasm(rwasm_store) => {
                        // A special case for rWasm, we need to reset state
                        rwasm_store.reset(false);
                    }
                    _ => {}
                }
                store.context_mut(|context_ref| *context_ref = runtime_context.clone());
            }

            // If there is no cached store, then construct a new one (slow)
            let store = store.unwrap_or_else(|| {
                strategy.create_store(
                    caching_runtime.import_linker.clone(),
                    runtime_context,
                    runtime_syscall_handler,
                )
            });

            Self {
                strategy,
                store,
                code_hash,
            }
        })
    }

    #[tracing::instrument(level = "info", skip_all)]
    pub fn execute(&mut self) -> ExecutionResult {
        let (fuel_limit, disable_fuel) =
            self.store.context(|ctx| (ctx.fuel_limit, ctx.disable_fuel));
        let result = self.execute_inner(Some(fuel_limit));
        let fuel_limit = if disable_fuel { None } else { Some(fuel_limit) };
        self.handle_execution_result(result, fuel_limit)
    }

    fn execute_inner(&mut self, fuel: Option<u64>) -> Result<(), TrapCode> {
        let state = self.store.context(|ctx| ctx.state);
        let func_name = match state {
            STATE_MAIN => "main",
            STATE_DEPLOY => "deploy",
            _ => unreachable!(),
        };
        self.strategy
            .execute(&mut self.store, func_name, &[], &mut [], fuel)
    }

    #[tracing::instrument(level = "info", skip_all, fields(fuel_ptr = fuel16_ptr, exit_code = exit_code))]
    pub fn resume(
        &mut self,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult {
        let disable_fuel = self.store.context(|ctx| ctx.is_fuel_disabled());
        let mut fuel_remaining = self.store.remaining_fuel();
        if disable_fuel {
            fuel_remaining = None;
        }
        let result = self.resume_inner(fuel16_ptr, fuel_consumed, fuel_refunded, exit_code);
        self.handle_execution_result(result, fuel_remaining)
    }

    fn resume_inner(
        &mut self,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> Result<(), TrapCode> {
        if fuel16_ptr > 0 {
            let mut buffer = [0u8; 16];
            LittleEndian::write_u64(&mut buffer[..8], fuel_consumed);
            LittleEndian::write_i64(&mut buffer[8..], fuel_refunded);
            self.store.memory_write(fuel16_ptr as usize, &buffer)?;
        }
        self.strategy
            .resume(&mut self.store, &[Value::I32(exit_code)], &mut [])
    }

    pub fn warmup_strategy(bytecode: Bytes, hash: B256, address: Address) {
        // save the current runtime state for future recovery
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            caching_runtime.get_module_or_init(BytecodeOrHash::Bytecode {
                address,
                bytecode,
                hash,
            });
        })
    }

    pub(crate) fn remember_runtime(self, _root_ctx: &mut RuntimeContext) -> i32 {
        // save the current runtime state for future recovery
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            let call_id = caching_runtime.transaction_call_id_counter;
            caching_runtime.transaction_call_id_counter += 1;
            // root_ctx.call_counter += 1;
            // let call_id = root_ctx.call_counter;
            caching_runtime.recoverable_runtimes.insert(call_id, self);
            call_id as i32
        })
    }

    pub(crate) fn return_store(self) {
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            if let Some(cached_module) = caching_runtime.cached_modules.get_mut(&self.code_hash) {
                cached_module.return_store(self.store);
            }
        });
    }

    pub(crate) fn recover_runtime(call_id: u32) -> Runtime {
        CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
            caching_runtime
                .recoverable_runtimes
                .remove(&call_id)
                .expect("runtime: can't resolve runtime by id, it should never happen")
        })
    }

    #[tracing::instrument(level = "info", skip_all)]
    fn handle_execution_result(
        &mut self,
        next_result: Result<(), TrapCode>,
        fuel_consumed_before_the_call: Option<u64>,
    ) -> ExecutionResult {
        let mut execution_result = self
            .store
            .context_mut(|ctx| take(&mut ctx.execution_result));
        // Once fuel is calculated, we must adjust our fuel limit,
        // because we don't know what gas conversion policy is used,
        // if there is rounding then it can cause miscalculations
        if let Some(fuel_consumed_before_the_call) = fuel_consumed_before_the_call {
            let diff = fuel_consumed_before_the_call - self.store.remaining_fuel().unwrap();
            execution_result.fuel_consumed = diff;
        }
        loop {
            match next_result {
                Ok(_) => break,
                Err(TrapCode::InterruptionCalled) => {
                    let resumable_context = self
                        .store
                        .context_mut(|ctx| ctx.resumable_context.take().unwrap());
                    if resumable_context.is_root {
                        unimplemented!("validate this logic, might not be ok in STF mode");
                    }
                    self.handle_resumable_state(&mut execution_result, resumable_context);
                    break;
                }
                Err(err) => {
                    execution_result.exit_code = ExitCode::from(err).into_i32();
                    break;
                }
            }
        }
        execution_result
    }

    fn handle_resumable_state(
        &mut self,
        execution_result: &mut ExecutionResult,
        sys_exec_resumable: InterruptionHolder,
    ) {
        // we disallow nested calls at non-root levels,
        // so we must save the current state
        // to interrupt execution and delegate decision-making
        // to the root execution
        self.store.context_mut(|ctx| {
            let output = &mut ctx.execution_result.output;
            output.clear();
            assert!(output.is_empty(), "runtime: return data must be empty");
        });
        // serialize the delegated execution state,
        // but we don't serialize registers and stack state,
        // instead we remember it inside the internal structure
        // and assign a special identifier for recovery
        execution_result.output = sys_exec_resumable.params.encode();
        // interruption is a special exit code that indicates to the root what happened inside
        // the call
        execution_result.interrupted = true;
    }
}

// TODO(dmitry123): This one is dirty, but we call this function from Reth to reset `call_id` counter
//  before executing new transaction. We should pass `call_id` using root context.
pub fn reset_call_id_counter() {
    CACHING_RUNTIME_FACTORY.with_borrow_mut(|caching_runtime| {
        caching_runtime.transaction_call_id_counter = 1;
        caching_runtime.recoverable_runtimes.clear();
    });
}
