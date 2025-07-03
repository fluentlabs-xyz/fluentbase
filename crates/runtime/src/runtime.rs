use crate::{
    context::RuntimeContext,
    instruction::{exec::SysExecResumable, invoke_runtime_handler},
};
use fluentbase_codec::{bytes::BytesMut, CompactABI};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    create_import_linker,
    is_resumable_precompile,
    Address,
    BytecodeOrHash,
    Bytes,
    ExitCode,
    SysFuncIdx,
    B256,
    PRECOMPILE_ADDRESSES,
    STATE_DEPLOY,
    STATE_MAIN,
};
use hashbrown::{hash_map::Entry, HashMap};
use rwasm::{
    ExecutionEngine,
    ExecutorConfig,
    ImportLinker,
    RwasmModule,
    Store,
    Strategy,
    TrapCode,
    TypedCaller,
    TypedStore,
    Value,
};
use std::{cell::RefCell, fmt::Debug, mem::take, rc::Rc, sync::Arc};

#[derive(Default, Clone, Debug)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub fuel_consumed: u64,
    pub fuel_refunded: i64,
    pub return_data: Vec<u8>,
    pub output: Vec<u8>,
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
    call_id_counter: u32,
}

impl CachingRuntime {
    pub fn new() -> Self {
        Self {
            strategies: HashMap::new(),
            recoverable_runtimes: HashMap::new(),
            import_linker: create_import_linker(),
            call_id_counter: 1,
        }
    }

    pub fn init_strategy(
        &mut self,
        address: Address,
        rwasm_bytecode: Bytes,
        code_hash: B256,
    ) -> Arc<Strategy> {
        let entry = match self.strategies.entry(code_hash) {
            Entry::Occupied(_) => unreachable!("runtime: unloaded module"),
            Entry::Vacant(entry) => entry,
        };
        let rwasm_module = Rc::new(RwasmModule::new_or_empty(rwasm_bytecode.as_ref()).0);
        #[cfg(feature = "wasmtime")]
        if PRECOMPILE_ADDRESSES.contains(&address) {
            let wasmtime_module =
                rwasm::compile_wasmtime_module(&rwasm_module.wasm_section).unwrap();
            let strategy = Arc::new(Strategy::Wasmtime {
                module: Rc::new(wasmtime_module),
                resumable: is_resumable_precompile(&address),
            });
            entry.insert(strategy.clone());
            return strategy;
        }
        let strategy = Arc::new(Strategy::Rwasm {
            module: rwasm_module,
            engine: ExecutionEngine::acquire_shared(),
        });
        entry.insert(strategy.clone());
        strategy
    }

    pub fn resolve_strategy(&self, rwasm_hash: &B256) -> Option<Arc<Strategy>> {
        self.strategies.get(rwasm_hash).cloned()
    }
}

thread_local! {
    static CACHING_RUNTIME: RefCell<CachingRuntime> = RefCell::new(CachingRuntime::new());
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
    pub fn catch_trap(err: &TrapCode) -> i32 {
        let err = match err {
            err => err,
        };
        ExitCode::from(err).into_i32()
    }

    pub fn run_with_context(runtime_context: RuntimeContext) -> ExecutionResult {
        Self::new(runtime_context).call()
    }

    pub fn new(runtime_context: RuntimeContext) -> Self {
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            // resolve cached module or init it
            let strategy = match runtime_context.bytecode.clone() {
                BytecodeOrHash::Bytecode {
                    address,
                    rwasm_module,
                    code_hash,
                } => {
                    // if we have a cached module, then use it, otherwise create a new one and cache
                    if let Some(existing_strategy) = caching_runtime.resolve_strategy(&code_hash) {
                        existing_strategy
                    } else {
                        caching_runtime.init_strategy(
                            address,
                            rwasm_module.clone(),
                            code_hash.clone(),
                        )
                    }
                }
                BytecodeOrHash::Hash(_hash) => {
                    panic!("runtime: can't run just by hash")
                }
            };

            let config = ExecutorConfig::new()
                .fuel_limit(runtime_context.fuel_limit)
                .fuel_enabled(!runtime_context.disable_fuel);

            let store = strategy.create_store(
                config,
                caching_runtime.import_linker.clone(),
                runtime_context,
                runtime_syscall_handler,
            );

            Self { strategy, store }
        })
    }

    pub fn call(&mut self) -> ExecutionResult {
        let fuel_remaining = self.store.remaining_fuel();
        let fuel_refunded_before_the_call =
            self.store.context(|ctx| ctx.execution_result.fuel_refunded);
        let func_name = match self.store.context(|ctx| ctx.state) {
            STATE_MAIN => "main",
            STATE_DEPLOY => "deploy",
            _ => unreachable!(),
        };
        let result = self
            .strategy
            .execute(&mut self.store, func_name, &[], &mut []);
        self.handle_execution_result(result, fuel_remaining, fuel_refunded_before_the_call)
    }

    pub fn resume(
        &mut self,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult {
        let fuel_remaining_before_the_call = self.store.remaining_fuel();
        let fuel_refunded_before_the_call =
            self.store.context(|ctx| ctx.execution_result.fuel_refunded);

        let mut memory_changes: Vec<(u32, Box<[u8]>)> = Vec::with_capacity(1);
        if fuel16_ptr > 0 {
            let mut buffer = [0u8; 16];
            LittleEndian::write_u64(&mut buffer[..8], fuel_consumed);
            LittleEndian::write_i64(&mut buffer[8..], fuel_refunded);
            memory_changes.push((fuel16_ptr, buffer.into()));
        }

        let result = self.strategy.resume_wth_memory(
            &mut self.store,
            &[Value::I32(exit_code)],
            &mut [],
            memory_changes,
        );

        self.handle_execution_result(
            result,
            fuel_remaining_before_the_call,
            fuel_refunded_before_the_call,
        )
    }

    pub(crate) fn remember_runtime(self, _root_ctx: &mut RuntimeContext) -> i32 {
        // save the current runtime state for future recovery
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            let call_id = caching_runtime.call_id_counter;
            caching_runtime.call_id_counter += 1;
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

    fn handle_execution_result(
        &mut self,
        next_result: Result<(), TrapCode>,
        fuel_consumed_before_the_call: Option<u64>,
        fuel_refunded_before_the_call: i64,
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
        execution_result.fuel_refunded =
            execution_result.fuel_refunded - fuel_refunded_before_the_call;
        loop {
            match next_result {
                Ok(_) => break,
                Err(TrapCode::InterruptionCalled) => {
                    let resumable_context = self
                        .store
                        .context_mut(|ctx| ctx.resumable_context.take().unwrap());
                    if resumable_context.is_root {
                        unimplemented!("resumable context is root");
                        // // TODO(dmitry123): "validate this logic, might not be ok in STF
                        // mode" let (_, _, exit_code) =
                        // SyscallExec::fn_continue(
                        //     Caller::new(&mut self.executor),
                        //     &resumable_context,
                        // );
                        // next_result = Ok(exit_code);
                        // continue;
                    }
                    self.handle_resumable_state(&mut execution_result, &resumable_context);
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
        sys_exec_resumable: &SysExecResumable,
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
        let mut encoded_state = BytesMut::new();
        CompactABI::encode(&sys_exec_resumable.params, &mut encoded_state, 0)
            .expect("runtime: can't encode resumable state");
        execution_result
            .output
            .extend(encoded_state.freeze().to_vec());
        // interruption is a special exit code that indicates to the root what happened inside
        // the call
        execution_result.interrupted = true;
    }
}

pub fn reset_call_id_counter() {
    CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
        caching_runtime.call_id_counter = 1;
        caching_runtime.recoverable_runtimes.clear();
    });
}
