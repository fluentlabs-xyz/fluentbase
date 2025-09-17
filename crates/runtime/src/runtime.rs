use crate::{
    context::RuntimeContext,
    instruction::{exec::SysExecResumable, invoke_runtime_handler},
};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    create_import_linker, Address, BytecodeOrHash, Bytes, ExitCode, SysFuncIdx, B256, STATE_DEPLOY,
    STATE_MAIN,
};
use hashbrown::{hash_map::Entry, HashMap};
use rwasm::{
    ExecutionEngine, ImportLinker, RwasmModule, Store, Strategy, TrapCode, TypedCaller, TypedStore,
    Value,
};
use std::{cell::RefCell, fmt::Debug, mem::take, rc::Rc, sync::Arc};

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

impl ExecutionResult {
    pub fn new_error(exit_code: i32) -> Self {
        Self {
            exit_code,
            ..Default::default()
        }
    }
}

pub struct CachingRuntime {
    // TODO(dmitry123): add LRU cache to this map to avoid memory leak (or remove HashMap?)
    strategies: HashMap<B256, Arc<Strategy>>,
    recoverable_runtimes: HashMap<u32, Runtime>,
    import_linker: Rc<ImportLinker>,
    transaction_call_id_counter: u32,
}

thread_local! {
    static CACHING_RUNTIME: RefCell<CachingRuntime> = RefCell::new(CachingRuntime::new());
}

impl CachingRuntime {
    pub fn new() -> Self {
        Self {
            strategies: HashMap::new(),
            recoverable_runtimes: HashMap::new(),
            import_linker: create_import_linker(),
            transaction_call_id_counter: 1,
        }
    }

    #[tracing::instrument(level = "info", skip_all, fields(address = %address, code_hash = %code_hash))]
    pub fn init_strategy(
        &mut self,
        address: Address,
        bytecode: Bytes,
        code_hash: B256,
    ) -> Arc<Strategy> {
        let entry = match self.strategies.entry(code_hash) {
            Entry::Occupied(entry) => {
                let strategy = entry.get().clone();
                // strategy
                //     .store
                //     .borrow_mut()
                //     .context_mut(move |context_ref| *context_ref = runtime_context);
                return strategy;
            }
            Entry::Vacant(entry) => entry,
        };

        let _span = tracing::info_span!("parse_rwasm_module").entered();
        let rwasm_module = Rc::new(RwasmModule::new_or_empty(bytecode.as_ref()).0);
        drop(_span);

        #[cfg(feature = "wasmtime")]
        if fluentbase_types::is_system_precompile(&address) {
            let _span = tracing::info_span!("compile_wasmtime_module").entered();
            let wasmtime_module = {
                #[cfg(feature = "inter-process-lock")]
                let _lock = crate::inter_process_lock::InterProcessLock::acquire_on_b256(
                    crate::inter_process_lock::FILE_NAME_PREFIX1,
                    &code_hash,
                )
                .unwrap();
                let config =
                    fluentbase_types::default_compilation_config().with_consume_fuel(false);
                let wasmtime_module =
                    rwasm::compile_wasmtime_module(config, &rwasm_module.hint_section).unwrap();
                wasmtime_module
            };
            let strategy = Strategy::Wasmtime {
                module: Rc::new(wasmtime_module),
            };
            return entry.insert(Arc::new(strategy)).clone();
        }

        #[cfg(not(feature = "wasmtime"))]
        let _ = address; // silence unused variable warning
        let strategy = Strategy::Rwasm {
            module: rwasm_module,
            engine: ExecutionEngine::acquire_shared(),
        };
        entry.insert(Arc::new(strategy)).clone()
    }
}

#[derive(Default)]
pub struct RuntimeSyscallHandler {}

fn runtime_syscall_handler(
    caller: &mut TypedCaller<RuntimeContext>,
    func_idx: u32,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    let sys_func_idx = SysFuncIdx::from_repr(func_idx).ok_or(TrapCode::UnknownExternalFunction)?;
    invoke_runtime_handler(caller, sys_func_idx, params, result)
}

pub struct Runtime {
    pub strategy: Arc<Strategy>,
    pub store: TypedStore<RuntimeContext>,
}

impl Runtime {
    #[deprecated(note = "use `new` method instead")]
    pub fn run_with_context(
        bytecode_or_hash: BytecodeOrHash,
        runtime_context: RuntimeContext,
    ) -> ExecutionResult {
        Self::new(bytecode_or_hash, runtime_context).execute(None)
    }

    #[tracing::instrument(level = "info", skip_all)]
    pub fn new(bytecode_or_hash: BytecodeOrHash, runtime_context: RuntimeContext) -> Self {
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            // resolve cached module or init it
            let strategy = match bytecode_or_hash {
                BytecodeOrHash::Bytecode {
                    address,
                    rwasm_module,
                    code_hash,
                } => {
                    // if we have a cached module, then use it, otherwise create a new one and cache
                    caching_runtime.init_strategy(address, rwasm_module, code_hash)
                }
                BytecodeOrHash::Hash(_hash) => {
                    panic!("runtime: can't run just by hash")
                }
            };

            let store = strategy.create_store(
                caching_runtime.import_linker.clone(),
                runtime_context,
                runtime_syscall_handler,
            );

            Self { strategy, store }
        })
    }

    #[tracing::instrument(level = "info", skip_all)]
    pub fn execute(&mut self, fuel: Option<u64>) -> ExecutionResult {
        let result = self.execute_inner(fuel);
        self.handle_execution_result(result, fuel)
    }

    fn execute_inner(&mut self, fuel: Option<u64>) -> Result<(), TrapCode> {
        let func_name = match self.store.context(|ctx| ctx.state) {
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
        let fuel_remaining = self.store.remaining_fuel();
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

    pub fn warmup_strategy(address: Address, rwasm_bytecode: Bytes, code_hash: B256) {
        // save the current runtime state for future recovery
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            caching_runtime.init_strategy(address, rwasm_bytecode, code_hash);
        })
    }

    pub(crate) fn remember_runtime(self, _root_ctx: &mut RuntimeContext) -> i32 {
        // save the current runtime state for future recovery
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            let call_id = caching_runtime.transaction_call_id_counter;
            caching_runtime.transaction_call_id_counter += 1;
            // root_ctx.call_counter += 1;
            // let call_id = root_ctx.call_counter;
            caching_runtime.recoverable_runtimes.insert(call_id, self);
            call_id as i32
        })
    }

    pub(crate) fn recover_runtime(call_id: u32) -> Runtime {
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
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
        // once fuel is calculated, we must adjust our fuel limit,
        // because we don't know what gas conversion policy is used,
        // if there is rounding then it can cause miscalculations
        if let Some(fuel_consumed_before_the_call) = fuel_consumed_before_the_call {
            execution_result.fuel_consumed =
                fuel_consumed_before_the_call - self.store.remaining_fuel().unwrap();
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
        sys_exec_resumable: SysExecResumable,
    ) {
        // we disallow nested calls at non-root levels,
        // so we must save the current state
        // to interrupt execution and delegate decision-making
        // to the root execution
        self.store.context_mut(|ctx| {
            let output = ctx.output_mut();
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

pub fn reset_call_id_counter() {
    CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
        caching_runtime.transaction_call_id_counter = 1;
        caching_runtime.recoverable_runtimes.clear();
    });
}
