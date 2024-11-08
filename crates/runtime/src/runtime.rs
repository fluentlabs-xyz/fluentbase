use crate::{
    instruction::{
        exec::{SysExecResumable, SyscallExec},
        runtime_register_handlers,
    },
    types::{NonePreimageResolver, PreimageResolver, RuntimeError},
};
use fluentbase_codec::{bytes::BytesMut, FluentABI};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{
    create_import_linker,
    Bytes,
    ExitCode,
    SysFuncIdx::STATE,
    F254,
    POSEIDON_EMPTY,
    STATE_DEPLOY,
    STATE_MAIN,
};
use hashbrown::{hash_map::Entry, HashMap};
use rwasm::{
    core::ImportLinker,
    engine::{bytecode::Instruction, DropKeep, RwasmConfig, StateRouterConfig, Tracer},
    instruction_set,
    rwasm::RwasmModule,
    AsContextMut,
    Caller,
    Engine,
    FuelConsumptionMode,
    Instance,
    Linker,
    Module,
    ResumableCall,
    ResumableInvocation,
    Store,
    Value,
};
use std::{
    cell::RefCell,
    fmt::{Debug, Formatter},
    mem::take,
    sync::atomic::{AtomicU32, Ordering},
};

#[derive(Clone)]
pub enum BytecodeOrHash {
    Bytecode(Bytes, Option<F254>),
    Hash(F254),
}

impl Default for BytecodeOrHash {
    fn default() -> Self {
        Self::Bytecode(Bytes::new(), Some(POSEIDON_EMPTY))
    }
}

impl BytecodeOrHash {
    pub fn with_resolved_hash(self) -> Self {
        match self {
            BytecodeOrHash::Bytecode(_, Some(_)) => self,
            BytecodeOrHash::Bytecode(bytecode, None) => {
                let hash = F254::from(poseidon_hash(&bytecode));
                BytecodeOrHash::Bytecode(bytecode, Some(hash))
            }
            BytecodeOrHash::Hash(_) => self,
        }
    }

    pub fn resolve_hash(&self) -> F254 {
        match self {
            BytecodeOrHash::Bytecode(_, hash) => hash.expect("poseidon hash must be resolved"),
            BytecodeOrHash::Hash(hash) => *hash,
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
    pub(crate) input: Vec<u8>,
    pub(crate) disable_fuel: bool,
    // context outputs
    pub(crate) execution_result: ExecutionResult,
    pub(crate) resumable_invocation: Option<ResumableInvocation>,
    pub(crate) instance: Option<Instance>,
    pub(crate) preimage_resolver: Box<dyn PreimageResolver>,
}

// pub type RuntimeContext = RuntimeContextFull<'static, ()>;

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
            input: vec![],
            call_depth: 0,
            trace: false,
            execution_result: ExecutionResult::default(),
            resumable_invocation: None,
            instance: None,
            disable_fuel: false,
            preimage_resolver: Default::default(),
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

    pub fn with_input(mut self, input_data: Vec<u8>) -> Self {
        self.input = input_data;
        self
    }

    pub fn change_input(&mut self, input_data: Vec<u8>) {
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

    pub fn with_preimage_resolver(mut self, preimage_resolver: Box<dyn PreimageResolver>) -> Self {
        self.preimage_resolver = preimage_resolver;
        self
    }

    pub fn set_preimage_resolver(&mut self, preimage_resolver: Box<dyn PreimageResolver>) {
        self.preimage_resolver = preimage_resolver;
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

    pub fn preimage_resolver(&self) -> &dyn PreimageResolver {
        self.preimage_resolver.as_ref()
    }

    pub fn depth(&self) -> u32 {
        self.call_depth
    }

    pub fn exit_code(&self) -> i32 {
        self.execution_result.exit_code
    }

    pub fn input(&self) -> &Vec<u8> {
        self.input.as_ref()
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

static mut GLOBAL_RUNTIME_INDEX: AtomicU32 = AtomicU32::new(1);

pub(crate) struct RecoverableRuntime {
    pub(crate) runtime: Runtime,
}

impl RecoverableRuntime {
    pub(crate) fn resumable_invocation(&self) -> &ResumableInvocation {
        self.runtime
            .data()
            .resumable_invocation
            .as_ref()
            .expect("missing resumable context")
    }

    pub(crate) fn state(&self) -> &SysExecResumable {
        self.resumable_invocation()
            .host_error()
            .downcast_ref::<SysExecResumable>()
            .expect("can't downcast resumable invocation")
    }
}

pub struct CachingRuntime {
    // TODO(dmitry123): "add expiration to this map to avoid memory leak"
    cached_bytecode: HashMap<F254, Bytes>,
    modules: HashMap<F254, Module>,
    recoverable_runtimes: HashMap<u32, RecoverableRuntime>,
}

impl CachingRuntime {
    pub fn new() -> Self {
        Self {
            cached_bytecode: HashMap::new(),
            modules: HashMap::new(),
            recoverable_runtimes: HashMap::new(),
        }
    }

    fn new_engine(disable_fuel: bool) -> Engine {
        // we can safely use sovereign import linker because all protected are filtered out during
        // a translation process
        let import_linker = Runtime::new_import_linker();
        let mut config = RwasmModule::default_config(None);
        config.rwasm_config(RwasmConfig {
            state_router: Some(StateRouterConfig {
                states: Box::new([
                    ("deploy".to_string(), STATE_DEPLOY),
                    ("main".to_string(), STATE_MAIN),
                ]),
                opcode: Instruction::Call(STATE.into()),
            }),
            entrypoint_name: None,
            import_linker: Some(import_linker),
            wrap_import_functions: true,
        });
        config
            .floats(false)
            .fuel_consumption_mode(FuelConsumptionMode::Eager)
            .consume_fuel(!disable_fuel);
        Engine::new(&config)
    }

    pub fn init_module(
        &mut self,
        engine: &Engine,
        rwasm_hash: F254,
    ) -> Result<&Module, RuntimeError> {
        let rwasm_bytecode = self
            .cached_bytecode
            .get(&rwasm_hash)
            .expect("missing cached rWASM bytecode");
        let entry = match self.modules.entry(rwasm_hash) {
            Entry::Occupied(_) => return Err(RuntimeError::UnloadedModule(rwasm_hash)),
            Entry::Vacant(entry) => entry,
        };
        // empty bytecode we can't execute, so return Ok exit code
        let reduced_module = if !rwasm_bytecode.is_empty() {
            RwasmModule::new(rwasm_bytecode).map_err(Into::<RuntimeError>::into)?
        } else {
            RwasmModule::from(instruction_set! {
                Return(DropKeep::none())
            })
        };
        let module_builder = reduced_module.to_module_builder(engine);
        let module = module_builder.finish();
        Ok(entry.insert(module))
    }

    pub fn resolve_module(&self, rwasm_hash: &F254) -> Option<&Module> {
        self.modules.get(rwasm_hash)
    }
}

thread_local! {
    static CACHING_RUNTIME: RefCell<CachingRuntime> = RefCell::new(CachingRuntime::new());
}

pub struct Runtime {
    // store and linker
    pub(crate) store: Store<RuntimeContext>,
    pub(crate) linker: Linker<RuntimeContext>,
}

impl Runtime {
    pub fn new_import_linker() -> ImportLinker {
        create_import_linker()
    }

    pub fn catch_trap(err: &RuntimeError) -> i32 {
        let err = match err {
            RuntimeError::Rwasm(err) => err,
            RuntimeError::ExitCode(exit_code) => return *exit_code,
            _ => return ExitCode::UnknownError as i32,
        };
        let err = match err {
            rwasm::Error::Trap(err) => err,
            _ => {
                println!("{:?}", err);
                return ExitCode::UnknownError as i32;
            }
        };
        // for i32 error code (raw error) just return result
        if let Some(exit_status) = err.i32_exit_status() {
            return exit_status;
        }
        // for trap code (wasmi error) convert error to i32
        if let Some(trap_code) = err.trap_code() {
            return Into::<ExitCode>::into(trap_code) as i32;
        }
        // otherwise it's just an unknown error
        ExitCode::UnknownError as i32
    }

    pub fn run_with_context(runtime_context: RuntimeContext) -> ExecutionResult {
        Self::new(runtime_context).call()
    }

    pub fn new(mut runtime_context: RuntimeContext) -> Self {
        // make sure bytecode hash is resolved
        runtime_context.bytecode = runtime_context.bytecode.with_resolved_hash();

        // use existing engine or create a new one
        let engine = CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            let rwasm_hash = runtime_context.bytecode.resolve_hash();
            caching_runtime
                .resolve_module(&rwasm_hash)
                .map(|module| module.engine.clone())
                .unwrap_or_else(|| CachingRuntime::new_engine(runtime_context.disable_fuel))
        });

        // create new linker and store (it shares the same engine resources)
        let mut store = if runtime_context.trace {
            Store::<RuntimeContext>::new(&engine, runtime_context).with_tracer(Tracer::default())
        } else {
            Store::<RuntimeContext>::new(&engine, runtime_context)
        };
        let mut linker = Linker::<RuntimeContext>::new(&engine);

        // add fuel if the limit is specified
        if store.engine().config().get_consume_fuel() {
            store
                .add_fuel(store.data().fuel_limit)
                .expect("fuel metering is disabled");
        }

        // register linker trampolines for external calls
        runtime_register_handlers(&mut linker, &mut store);

        Self { store, linker }
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

    fn instantiate_module<PR: PreimageResolver>(
        &mut self,
        preimage_resolver: &PR,
    ) -> Result<Instance, RuntimeError> {
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            let bytecode_repr = take(&mut self.store.data_mut().bytecode);

            // resolve cached module or init it
            let module = match &bytecode_repr {
                BytecodeOrHash::Bytecode(bytecode, hash) => {
                    let hash = hash.unwrap_or_else(|| F254::from(poseidon_hash(&bytecode)));
                    // if we have a cached module then use it, otherwise create a new one and cache
                    if let Some(module) = caching_runtime.resolve_module(&hash) {
                        Ok(module)
                    } else {
                        caching_runtime
                            .cached_bytecode
                            .insert(hash, bytecode.clone());
                        caching_runtime.init_module(self.store.engine(), hash)
                    }
                }
                BytecodeOrHash::Hash(hash) => {
                    // if we have only hash, then try to load module or fail fast
                    match caching_runtime.resolve_module(hash) {
                        Some(module) => Ok(module),
                        None => {
                            let cached_bytecode = caching_runtime.cached_bytecode.get(hash);
                            if cached_bytecode.is_none() {
                                let bytecode = preimage_resolver.preimage(hash).unwrap_or_default();
                                caching_runtime
                                    .cached_bytecode
                                    .insert(*hash, bytecode.into());
                            }
                            caching_runtime.init_module(self.store.engine(), *hash)
                        }
                    }
                }
            }?;

            // return bytecode
            self.store.data_mut().bytecode = bytecode_repr;

            // init instance
            let instance = self
                .linker
                .instantiate(&mut self.store, &module)
                .map_err(Into::<RuntimeError>::into)?
                .start(&mut self.store)
                .map_err(Into::<RuntimeError>::into)?;

            Ok::<Instance, RuntimeError>(instance)
        })
    }

    pub fn call(&mut self) -> ExecutionResult {
        let fuel_consumed_before_call = self.store.fuel_consumed().unwrap_or_default();
        match self.call_internal(fuel_consumed_before_call, &NonePreimageResolver) {
            Ok(result) => result,
            Err(err) => self.handle_execution_result(Some(err), fuel_consumed_before_call),
        }
    }

    pub fn call_with_preimage_resolver<PR: PreimageResolver>(
        &mut self,
        preimage_resolver: &PR,
    ) -> ExecutionResult {
        let fuel_consumed_before_call = self.store.fuel_consumed().unwrap_or_default();
        match self.call_internal(fuel_consumed_before_call, preimage_resolver) {
            Ok(result) => result,
            Err(err) => self.handle_execution_result(Some(err), fuel_consumed_before_call),
        }
    }

    fn call_internal<PR: PreimageResolver>(
        &mut self,
        fuel_consumed_before_call: u64,
        preimage_resolver: &PR,
    ) -> Result<ExecutionResult, RuntimeError> {
        let instance = self.instantiate_module(preimage_resolver)?;
        let next_result = instance
            .get_func(&mut self.store, "main")
            .ok_or(RuntimeError::MissingEntrypoint)?
            .call_resumable(&mut self.store, &[], &mut [])
            .map_err(Into::<RuntimeError>::into);
        self.handle_resumable_call_result(next_result, fuel_consumed_before_call, instance)
    }

    pub fn resume(&mut self, exit_code: i32, fuel_consumed_before_call: u64) -> ExecutionResult {
        let resumable_invocation = self
            .store_mut()
            .data_mut()
            .resumable_invocation
            .take()
            .expect("can't resolve resumable invocation");
        let instance = self
            .store_mut()
            .data_mut()
            .instance
            .take()
            .expect("can't resolve instance");
        match self.resume_internal(
            resumable_invocation,
            instance,
            exit_code,
            fuel_consumed_before_call,
        ) {
            Ok(result) => result,
            Err(err) => self.handle_execution_result(Some(err), fuel_consumed_before_call),
        }
    }

    fn resume_internal(
        &mut self,
        resumable_invocation: ResumableInvocation,
        instance: Instance,
        exit_code: i32,
        fuel_consumed_before_call: u64,
    ) -> Result<ExecutionResult, RuntimeError> {
        let exit_code = Value::I32(exit_code);
        let next_result = resumable_invocation
            .resume(self.store.as_context_mut(), &[exit_code], &mut [])
            .map_err(Into::<RuntimeError>::into);
        self.handle_resumable_call_result(next_result, fuel_consumed_before_call, instance)
    }

    fn handle_resumable_call_result(
        &mut self,
        mut next_result: Result<ResumableCall, RuntimeError>,
        fuel_consumed_before_call: u64,
        instance: Instance,
    ) -> Result<ExecutionResult, RuntimeError> {
        loop {
            let resumable_invocation = match next_result? {
                ResumableCall::Finished => {
                    return Ok(self.handle_execution_result(None, fuel_consumed_before_call))
                }
                ResumableCall::Resumable(state) => state,
            };

            // if we have an exit code then return it, somehow execution failed, maybe if was out of
            // fuel, memory out of bounds or stack overflow/underflow
            if let Some(exit_code) = resumable_invocation.host_error().i32_exit_status() {
                return Err(RuntimeError::ExitCode(exit_code));
            }

            // if we can't downcast our resumable invocation state, then something unexpected
            // happened, and we can only return an error,
            // but maybe the crash is safer
            if let Some(delayed_state) = resumable_invocation
                .host_error()
                .downcast_ref::<SysExecResumable>()
            {
                if !delayed_state.is_root {
                    return self.handle_resumable_state(resumable_invocation, instance.clone());
                }
                // if we're at zero depth level, then we can safely execute function
                // since this call is initiated on the root level and it is trusted
                let exit_code = SyscallExec::fn_continue(
                    Caller::new(&mut self.store, Some(&instance)),
                    delayed_state,
                )
                .unwrap_or_else(|exit_code| {
                    exit_code
                        .i32_exit_status()
                        .unwrap_or(ExitCode::UnknownError.into_i32())
                });
                let exit_code = Value::I32(exit_code);
                next_result = resumable_invocation
                    .resume(self.store.as_context_mut(), &[exit_code], &mut [])
                    .map_err(Into::<RuntimeError>::into);
            } else {
                let exit_code = ExitCode::from(resumable_invocation.host_error());
                return Err(RuntimeError::ExitCode(exit_code.into_i32()));
            };
        }
    }

    fn handle_resumable_state(
        &mut self,
        resumable_invocation: ResumableInvocation,
        instance: Instance,
    ) -> Result<ExecutionResult, RuntimeError> {
        let delayed_state = resumable_invocation
            .host_error()
            .downcast_ref::<SysExecResumable>()
            .unwrap();
        // we disallow nested calls at non-root levels
        // so we must save the current state
        // to interrupt execution and delegate decision-making
        // to the root execution
        let output = self.store.data_mut().output_mut();
        output.clear();
        assert!(output.is_empty(), "return data must be empty");
        // serialize delegated execution state,
        // but we don't serialize registers and stack state,
        // instead we remember it inside the internal structure
        // and assign a special identifier for recovery
        let mut encoded_state = BytesMut::new();
        FluentABI::encode(&delayed_state.params, &mut encoded_state, 0)
            .map_err(Into::<RuntimeError>::into)?;
        output.extend(encoded_state.freeze().to_vec());
        // save resumable invocation inside store
        self.store_mut().data_mut().resumable_invocation = Some(resumable_invocation);
        self.store_mut().data_mut().instance = Some(instance);
        // interruption is a special exit code that indicates to the root what happened inside
        // the call
        Err(RuntimeError::ExecutionInterrupted)
    }

    pub(crate) fn remember_runtime(self) -> u32 {
        // save the current runtime state for future recovery
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            let call_id = unsafe { GLOBAL_RUNTIME_INDEX.fetch_add(1u32, Ordering::Relaxed) };
            let recoverable_runtime = RecoverableRuntime { runtime: self };
            caching_runtime
                .recoverable_runtimes
                .insert(call_id, recoverable_runtime);
            call_id
        })
    }

    pub(crate) fn recover_runtime(call_id: u32) -> RecoverableRuntime {
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            caching_runtime
                .recoverable_runtimes
                .remove(&call_id)
                .expect("can't resolve runtime by id, it should never happen")
        })
    }

    fn handle_execution_result(
        &mut self,
        err: Option<RuntimeError>,
        fuel_consumed_before_call: u64,
    ) -> ExecutionResult {
        let mut execution_result = take(&mut self.store.data_mut().execution_result);
        execution_result.fuel_consumed =
            self.store.fuel_consumed().unwrap_or_default() - fuel_consumed_before_call;
        if let Some(err) = err {
            match err {
                RuntimeError::ExecutionInterrupted => execution_result.interrupted = true,
                _ => {
                    let exit_code = Runtime::catch_trap(&err);
                    if exit_code != ExitCode::ExecutionHalted.into_i32() {
                        execution_result.exit_code = exit_code
                    }
                }
            }
        }
        execution_result
    }

    pub fn store(&self) -> &Store<RuntimeContext> {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store<RuntimeContext> {
        &mut self.store
    }

    pub fn data(&self) -> &RuntimeContext {
        self.store.data()
    }

    pub fn data_mut(&mut self) -> &mut RuntimeContext {
        self.store.data_mut()
    }

    pub fn take_context(&mut self) -> RuntimeContext {
        take(self.store.data_mut())
    }
}
