use crate::{
    context::RuntimeContext,
    instruction::{
        exec::{SysExecResumable, SyscallExec},
        invoke_runtime_handler,
    },
};
use fluentbase_codec::{bytes::BytesMut, CompactABI};
use fluentbase_types::{
    byteorder::{ByteOrder, LittleEndian},
    BytecodeOrHash,
    Bytes,
    ExitCode,
    SysFuncIdx,
    B256,
};
use hashbrown::{hash_map::Entry, HashMap};
use rwasm::{
    make_instruction_table,
    Caller,
    ExecutorConfig,
    InstructionTable,
    RwasmError,
    RwasmExecutor,
    RwasmModule,
};
use std::{
    cell::RefCell,
    fmt::Debug,
    mem::take,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

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
    // TODO(dmitry123): "add LRU cache to this map to avoid memory leak"
    cached_bytecode: HashMap<B256, Bytes>,
    modules: HashMap<B256, Arc<RwasmModule>>,
    recoverable_runtimes: HashMap<u32, Runtime>,
}

impl CachingRuntime {
    pub fn new() -> Self {
        Self {
            cached_bytecode: HashMap::new(),
            modules: HashMap::new(),
            recoverable_runtimes: HashMap::new(),
        }
    }

    pub fn init_module(&mut self, rwasm_hash: B256) -> Arc<RwasmModule> {
        let rwasm_bytecode = self
            .cached_bytecode
            .get(&rwasm_hash)
            .expect("runtime: missing cached bytecode");
        let entry = match self.modules.entry(rwasm_hash) {
            Entry::Occupied(_) => unreachable!("runtime: unloaded module"),
            Entry::Vacant(entry) => entry,
        };
        let reduced_module = Arc::new(RwasmModule::new_or_empty(rwasm_bytecode));
        entry.insert(reduced_module.clone());
        reduced_module
    }

    pub fn resolve_module(&self, rwasm_hash: &B256) -> Option<Arc<RwasmModule>> {
        self.modules.get(rwasm_hash).cloned()
    }
}

thread_local! {
    static CACHING_RUNTIME: RefCell<CachingRuntime> = RefCell::new(CachingRuntime::new());
}

#[derive(Default)]
pub struct RuntimeSyscallHandler {}

fn runtime_syscall_handler(
    caller: Caller<RuntimeContext>,
    func_idx: u32,
) -> Result<(), RwasmError> {
    let sys_func_idx =
        SysFuncIdx::from_repr(func_idx).ok_or(RwasmError::UnknownExternalFunction(func_idx))?;
    invoke_runtime_handler(caller, sys_func_idx)
}

pub struct Runtime {
    pub executor: RwasmExecutor<RuntimeContext>,
}

const INSTRUCTION_TABLE: InstructionTable<RuntimeContext> = make_instruction_table();

pub(crate) static CALL_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

impl Runtime {
    pub fn catch_trap(err: &RwasmError) -> i32 {
        let err = match err {
            RwasmError::ExecutionHalted(exit_code) => return *exit_code,
            err => err,
        };
        ExitCode::from(err).into_i32()
    }

    pub fn run_with_context(runtime_context: RuntimeContext) -> ExecutionResult {
        Self::new(runtime_context).call()
    }

    pub fn new(mut runtime_context: RuntimeContext) -> Self {
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            // make sure bytecode hash is resolved
            runtime_context.bytecode = runtime_context.bytecode.with_resolved_hash();

            let bytecode_repr = take(&mut runtime_context.bytecode);

            // resolve cached module or init it
            let rwasm_module = match &bytecode_repr {
                BytecodeOrHash::Bytecode(bytecode, hash) => {
                    let hash = hash.unwrap();
                    // if we have a cached module then use it, otherwise create a new one and cache
                    if let Some(module) = caching_runtime.resolve_module(&hash) {
                        module
                    } else {
                        caching_runtime
                            .cached_bytecode
                            .insert(hash, bytecode.clone());
                        caching_runtime.init_module(hash)
                    }
                }
                BytecodeOrHash::Hash(hash) => {
                    // if we have only hash, then try to load module or fail fast
                    match caching_runtime.resolve_module(hash) {
                        Some(module) => module,
                        None => caching_runtime.init_module(*hash),
                    }
                }
            };

            // return bytecode
            runtime_context.bytecode = bytecode_repr;

            let mut executor = RwasmExecutor::new(
                rwasm_module.clone(),
                ExecutorConfig::new()
                    .fuel_limit(runtime_context.fuel_limit)
                    .trace_enabled(runtime_context.trace)
                    .fuel_enabled(!runtime_context.disable_fuel),
                runtime_context,
            );
            executor.set_syscall_handler(runtime_syscall_handler);
            Self { executor }
        })
    }

    pub fn is_warm_bytecode(hash: &B256) -> bool {
        CACHING_RUNTIME
            .with_borrow(|caching_runtime| caching_runtime.cached_bytecode.contains_key(hash))
    }

    pub fn warmup_bytecode(hash: B256, bytecode: Bytes) {
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            caching_runtime.cached_bytecode.insert(hash, bytecode);
        });
    }

    pub fn call(&mut self) -> ExecutionResult {
        let fuel_consumed_before_the_call = self.executor.fuel_consumed();
        let fuel_refunded_before_the_call = self.executor.fuel_refunded();
        let result = self.executor.run();
        self.handle_execution_result(
            result,
            fuel_consumed_before_the_call,
            fuel_refunded_before_the_call,
        )
    }

    pub fn resume(
        &mut self,
        fuel16_ptr: u32,
        fuel_consumed: u64,
        fuel_refunded: i64,
        exit_code: i32,
    ) -> ExecutionResult {
        let fuel_consumed_before_the_call = self.executor.fuel_consumed();
        let fuel_refunded_before_the_call = self.executor.fuel_refunded();
        let mut caller = Caller::new(&mut self.executor);
        if fuel16_ptr > 0 {
            let mut buffer = [0u8; 16];
            LittleEndian::write_u64(&mut buffer[..8], fuel_consumed);
            LittleEndian::write_i64(&mut buffer[8..], fuel_refunded);
            // if we can't write a result into memory, then process it as an error
            if let Err(err) = caller.memory_write(fuel16_ptr as usize, &buffer) {
                return self.handle_execution_result(
                    Err(err),
                    fuel_consumed_before_the_call,
                    fuel_refunded_before_the_call,
                );
            }
        }
        caller.stack_push(exit_code);
        let result = self.executor.run();
        self.handle_execution_result(
            result,
            fuel_consumed_before_the_call,
            fuel_refunded_before_the_call,
        )
    }

    pub(crate) fn remember_runtime(self, _root_ctx: &mut RuntimeContext) -> i32 {
        // save the current runtime state for future recovery
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            // TODO(dmitry123): "don't use global call counter"
            let call_id = CALL_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
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
        mut next_result: Result<i32, RwasmError>,
        fuel_consumed_before_the_call: u64,
        fuel_refunded_before_the_call: i64,
    ) -> ExecutionResult {
        let mut execution_result = take(&mut self.executor.context_mut().execution_result);
        // once fuel is calculated, we must adjust our fuel limit,
        // because we don't know what gas conversion policy is used,
        // if there is rounding then it can cause miscalculations
        execution_result.fuel_consumed =
            self.executor.fuel_consumed() - fuel_consumed_before_the_call;
        execution_result.fuel_refunded =
            self.executor.fuel_refunded() - fuel_refunded_before_the_call;
        loop {
            match next_result {
                Ok(exit_code) => {
                    if exit_code != ExitCode::Ok.into_i32() {
                        execution_result.exit_code = exit_code;
                    }
                    break;
                }
                Err(err) => match err {
                    RwasmError::MalformedBinary => {
                        unreachable!("runtime: binary format error is not possible here")
                    }
                    RwasmError::UnknownExternalFunction(func_idx) => {
                        unreachable!(
                            "runtime: unknown external function ({}) error is not possible here",
                            func_idx
                        )
                    }
                    RwasmError::ExecutionHalted(exit_code) => {
                        unreachable!(
                            "runtime: execution halted ({}) error must be unwrapped",
                            exit_code
                        )
                    }
                    RwasmError::HostInterruption(host_error) => {
                        let resumable_state = host_error
                            .downcast_ref::<SysExecResumable>()
                            .expect("runtime: invalid resumable state");

                        if resumable_state.is_root {
                            // TODO(dmitry123): "validate this logic, might not be ok in STF mode"
                            let (_, _, exit_code) = SyscallExec::fn_continue(
                                Caller::new(&mut self.executor),
                                resumable_state,
                            );
                            next_result = Ok(exit_code);
                            continue;
                        }

                        self.handle_resumable_state(&mut execution_result, resumable_state);
                        break;
                    }
                    RwasmError::FloatsAreDisabled => {
                        unreachable!("runtime: floats are disabled")
                    }
                    RwasmError::NotAllowedInFuelMode => {
                        unreachable!("runtime: now allowed in fuel mode")
                    }
                    err => {
                        execution_result.exit_code = ExitCode::from(err).into_i32();
                        break;
                    }
                },
            }
        }
        execution_result
    }

    fn handle_resumable_state(
        &mut self,
        execution_result: &mut ExecutionResult,
        sys_exec_resumable: &SysExecResumable,
    ) {
        // we disallow nested calls at non-root levels
        // so we must save the current state
        // to interrupt execution and delegate decision-making
        // to the root execution
        let output = self.executor.context_mut().output_mut();
        output.clear();
        assert!(output.is_empty(), "runtime: return data must be empty");
        // serialize delegated execution state,
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

    pub fn context(&self) -> &RuntimeContext {
        self.executor.context()
    }

    pub fn context_mut(&mut self) -> &mut RuntimeContext {
        self.executor.context_mut()
    }

    pub fn take_context(&mut self) -> RuntimeContext {
        take(self.executor.context_mut())
    }
}
