use crate::{
    instruction::{
        exec::{SysExecResumable, SyscallExec},
        invoke_runtime_handler,
    },
    types::{NonePreimageResolver, PreimageResolver},
};
use fluentbase_codec::{bytes::BytesMut, CompactABI};
use fluentbase_rwasm::{
    Caller,
    ExecutorConfig,
    RwasmContext,
    RwasmError,
    RwasmExecutor,
    RwasmModule,
    RwasmModuleInstance,
    SyscallHandler,
};
use fluentbase_types::{Bytes, ExitCode, SysFuncIdx, F254, POSEIDON_EMPTY};
use hashbrown::{hash_map::Entry, HashMap};
use keccak_hash::keccak;
use std::{
    cell::RefCell,
    fmt::{Debug, Formatter},
    mem::take,
};

#[derive(Clone, Debug)]
pub enum BytecodeOrHash {
    Bytecode(Bytes, Option<F254>),
    Hash(F254),
    Instance(RwasmModuleInstance, Bytes, Option<F254>),
}

impl Default for BytecodeOrHash {
    fn default() -> Self {
        Self::Bytecode(Bytes::new(), Some(POSEIDON_EMPTY))
    }
}

impl BytecodeOrHash {
    pub fn with_resolved_hash(self) -> Self {
        let get_hash = |bytecode: &[u8]| {
            let hash = keccak(bytecode.as_ref()).0;
            F254::from(hash)
        };
        match self {
            BytecodeOrHash::Bytecode(_, Some(_)) => self,
            BytecodeOrHash::Bytecode(bytecode, None) => {
                let hash = get_hash(bytecode.as_ref());
                BytecodeOrHash::Bytecode(bytecode, Some(hash))
            }
            BytecodeOrHash::Hash(_) => self,
            BytecodeOrHash::Instance(_, _, Some(_)) => self,
            BytecodeOrHash::Instance(instance, bytecode, None) => {
                let hash = get_hash(bytecode.as_ref());
                BytecodeOrHash::Instance(instance, bytecode, Some(hash))
            }
        }
    }

    pub fn resolve_hash(&self) -> F254 {
        match self {
            BytecodeOrHash::Bytecode(_, hash) => hash.expect("hash must be resolved"),
            BytecodeOrHash::Hash(hash) => *hash,
            BytecodeOrHash::Instance(_, _, hash) => hash.expect("hash must be resolved"),
        }
    }
}

pub struct RuntimeContext {
    // context inputs
    pub(crate) bytecode: BytecodeOrHash,
    pub(crate) fuel_limit: u64,
    pub(crate) state: u32,
    pub(crate) call_depth: u32,
    pub(crate) trace: bool,
    pub(crate) input: Bytes,
    pub(crate) disable_fuel: bool,
    pub(crate) call_counter: u32,
    // context outputs
    pub(crate) execution_result: ExecutionResult,
}

impl Debug for RuntimeContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime context")
    }
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self {
            bytecode: BytecodeOrHash::default(),
            fuel_limit: 0,
            state: 0,
            input: Bytes::default(),
            call_depth: 0,
            trace: false,
            execution_result: ExecutionResult::default(),
            disable_fuel: false,
            call_counter: 0,
        }
    }
}

impl RuntimeContext {
    pub fn root(fuel_limit: u64) -> Self {
        Self::default().with_fuel_limit(fuel_limit).with_depth(0)
    }

    pub fn new<I: Into<Bytes>>(bytecode: I) -> Self {
        Self {
            bytecode: BytecodeOrHash::Bytecode(bytecode.into(), None),
            ..Default::default()
        }
    }

    pub fn new_with_hash(bytecode_hash: F254) -> Self {
        Self {
            bytecode: BytecodeOrHash::Hash(bytecode_hash),
            ..Default::default()
        }
    }

    pub fn with_bytecode(mut self, bytecode: BytecodeOrHash) -> Self {
        self.bytecode = bytecode;
        self
    }

    pub fn with_input<I: Into<Bytes>>(mut self, input_data: I) -> Self {
        self.input = input_data.into();
        self
    }

    pub fn change_input(&mut self, input_data: Bytes) {
        self.input = input_data;
    }

    pub fn with_state(mut self, state: u32) -> Self {
        self.state = state;
        self
    }

    pub fn with_fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = fuel_limit;
        self
    }

    pub fn with_depth(mut self, depth: u32) -> Self {
        self.call_depth = depth;
        self
    }

    pub fn with_tracer(mut self) -> Self {
        self.trace = true;
        self
    }

    pub fn with_disable_fuel(mut self, disable_fuel: bool) -> Self {
        self.disable_fuel = disable_fuel;
        self
    }

    pub fn without_fuel(mut self) -> Self {
        self.disable_fuel = true;
        self
    }

    pub fn depth(&self) -> u32 {
        self.call_depth
    }

    pub fn exit_code(&self) -> i32 {
        self.execution_result.exit_code
    }

    pub fn input(&self) -> &Bytes {
        &self.input
    }

    pub fn input_size(&self) -> u32 {
        self.input.len() as u32
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.execution_result.output
    }

    pub fn output_mut(&mut self) -> &mut Vec<u8> {
        &mut self.execution_result.output
    }

    pub fn fuel_limit(&self) -> u64 {
        self.fuel_limit
    }

    pub fn return_data(&self) -> &Vec<u8> {
        &self.execution_result.return_data
    }

    pub fn return_data_mut(&mut self) -> &mut Vec<u8> {
        &mut self.execution_result.return_data
    }

    pub fn state(&self) -> u32 {
        self.state
    }

    pub fn clear_output(&mut self) {
        self.execution_result.output.clear();
    }
}

#[derive(Default, Clone, Debug)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub fuel_consumed: u64,
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
    // TODO(dmitry123): "add expiration to this map to avoid memory leak"
    cached_bytecode: HashMap<F254, Bytes>,
    modules: HashMap<F254, RwasmModuleInstance>,
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

    pub fn insert_module(
        &mut self,
        rwasm_hash: F254,
        instance: RwasmModuleInstance,
        bytecode: Bytes,
    ) -> &RwasmModuleInstance {
        self.cached_bytecode.insert(rwasm_hash, bytecode);
        let entry = match self.modules.entry(rwasm_hash) {
            Entry::Occupied(_) => unreachable!("runtime: unloaded module"),
            Entry::Vacant(entry) => entry,
        };
        entry.insert(instance)
    }

    pub fn init_module(&mut self, rwasm_hash: F254) -> &RwasmModuleInstance {
        let rwasm_bytecode = self
            .cached_bytecode
            .get(&rwasm_hash)
            .expect("runtime: missing cached bytecode");
        let entry = match self.modules.entry(rwasm_hash) {
            Entry::Occupied(_) => unreachable!("runtime: unloaded module"),
            Entry::Vacant(entry) => entry,
        };
        let reduced_module =
            RwasmModule::new_or_empty(rwasm_bytecode).expect("runtime: can't parse rwasm module");
        entry.insert(reduced_module.instantiate())
    }

    pub fn resolve_module(&self, rwasm_hash: &F254) -> Option<&RwasmModuleInstance> {
        self.modules.get(rwasm_hash)
    }
}

thread_local! {
    static CACHING_RUNTIME: RefCell<CachingRuntime> = RefCell::new(CachingRuntime::new());
}

#[derive(Default)]
pub struct RuntimeSyscallHandler {}

impl SyscallHandler<RuntimeContext> for RuntimeSyscallHandler {
    fn call_function(caller: Caller<RuntimeContext>, func_idx: u32) -> Result<(), RwasmError> {
        let sys_func_idx =
            SysFuncIdx::from_repr(func_idx).ok_or(RwasmError::UnknownExternalFunction(func_idx))?;
        invoke_runtime_handler(caller, sys_func_idx)
    }
}

pub struct Runtime {
    pub(crate) executor: RwasmExecutor<RuntimeSyscallHandler, RuntimeContext>,
}

impl Runtime {
    pub fn catch_trap(err: &RwasmError) -> i32 {
        let err = match err {
            RwasmError::TrapCode(err) => err,
            RwasmError::ExecutionHalted(exit_code) => return *exit_code,
            _ => return ExitCode::UnknownError as i32,
        };
        // for i32 error code (raw error) just return result
        ExitCode::from(err).into_i32()
    }

    pub fn run_with_context(runtime_context: RuntimeContext) -> ExecutionResult {
        Self::new(runtime_context, &NonePreimageResolver).call()
    }

    pub fn new<PR: PreimageResolver>(
        mut runtime_context: RuntimeContext,
        preimage_resolver: &PR,
    ) -> Self {
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
                        None => {
                            let cached_bytecode = caching_runtime.cached_bytecode.get(hash);
                            if cached_bytecode.is_none() {
                                let bytecode = preimage_resolver.preimage(hash).unwrap_or_default();
                                caching_runtime
                                    .cached_bytecode
                                    .insert(*hash, bytecode.into());
                            }
                            caching_runtime.init_module(*hash)
                        }
                    }
                }
                BytecodeOrHash::Instance(instance, bytecode, hash) => {
                    let hash = hash.unwrap();
                    // if we have a cached module then use it, otherwise create a new one and cache
                    if let Some(module) = caching_runtime.resolve_module(&hash) {
                        module
                    } else {
                        caching_runtime.insert_module(hash, instance.clone(), bytecode.clone())
                    }
                }
            };

            // return bytecode
            runtime_context.bytecode = bytecode_repr;

            let executor = RwasmExecutor::new(
                rwasm_module.clone(),
                ExecutorConfig::new()
                    .fuel_limit(runtime_context.fuel_limit)
                    .tracer_enabled(runtime_context.trace),
                runtime_context,
            );
            Self { executor }
        })
    }

    pub fn is_warm_bytecode(hash: &F254) -> bool {
        CACHING_RUNTIME
            .with_borrow(|caching_runtime| caching_runtime.cached_bytecode.contains_key(hash))
    }

    pub fn warmup_bytecode(hash: F254, bytecode: Bytes) {
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            caching_runtime.cached_bytecode.insert(hash, bytecode);
        });
    }

    pub fn call(&mut self) -> ExecutionResult {
        let fuel_consumed_before_call = self.executor.store().fuel_consumed();
        let result = self.executor.run();
        self.handle_execution_result(result, fuel_consumed_before_call)
    }

    pub fn resume(&mut self, exit_code: i32, fuel_consumed_before_call: u64) -> ExecutionResult {
        let mut caller = Caller::new(self.executor.store_mut());
        caller.stack_push(exit_code);
        let result = self.executor.run();
        self.handle_execution_result(result, fuel_consumed_before_call)
    }

    pub(crate) fn remember_runtime(self, root_ctx: &mut RuntimeContext) -> i32 {
        // save the current runtime state for future recovery
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            root_ctx.call_counter += 1;
            let call_id = root_ctx.call_counter;
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
        fuel_consumed_before_call: u64,
    ) -> ExecutionResult {
        let mut execution_result =
            take(&mut self.executor.store_mut().context_mut().execution_result);
        execution_result.fuel_consumed =
            self.executor.store().fuel_consumed() - fuel_consumed_before_call;
        loop {
            match next_result {
                Ok(exit_code) => {
                    if exit_code != ExitCode::ExecutionHalted.into_i32() {
                        execution_result.exit_code = exit_code;
                    }
                    break;
                }
                Err(err) => match err {
                    RwasmError::MalformedBinary => {
                        unreachable!("runtime: binary format error is not possible here")
                    }
                    RwasmError::TrapCode(trap_code) => {
                        execution_result.exit_code = ExitCode::from(trap_code).into_i32();
                        break;
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
                    RwasmError::MemoryError(_) => {
                        execution_result.exit_code = ExitCode::MemoryOutOfBounds.into_i32();
                        break;
                    }
                    RwasmError::HostInterruption(host_error) => {
                        let resumable_state = host_error
                            .downcast_ref::<SysExecResumable>()
                            .expect("runtime: invalid resumable state");

                        if resumable_state.is_root {
                            let exit_code = SyscallExec::fn_continue(
                                Caller::new(self.executor.store_mut()),
                                resumable_state,
                            )
                            .unwrap_or_else(|err| Runtime::catch_trap(&err));
                            let mut caller = Caller::new(self.executor.store_mut());
                            caller.stack_push(exit_code);
                            next_result = self.executor.run();
                            continue;
                        }

                        self.handle_resumable_state(&mut execution_result, resumable_state);
                        break;
                    }
                    RwasmError::FloatsAreDisabled => {
                        unreachable!("runtime: floats are disabled")
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
        let output = self.executor.store_mut().context_mut().output_mut();
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

    pub fn store(&self) -> &RwasmContext<RuntimeContext> {
        self.executor.store()
    }

    pub fn store_mut(&mut self) -> &mut RwasmContext<RuntimeContext> {
        self.executor.store_mut()
    }

    pub fn context(&self) -> &RuntimeContext {
        self.executor.store().context()
    }

    pub fn context_mut(&mut self) -> &mut RuntimeContext {
        self.executor.store_mut().context_mut()
    }

    pub fn take_context(&mut self) -> RuntimeContext {
        take(self.executor.store_mut().context_mut())
    }
}
