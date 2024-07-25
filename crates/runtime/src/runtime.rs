use crate::{
    instruction::{
        exec::{SysExecResumable, SyscallExec},
        runtime_register_shared_handlers,
        runtime_register_sovereign_handlers,
    },
    types::{InMemoryTrieDb, RuntimeError},
    zktrie::ZkTrieStateDb,
    JournaledTrie,
};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{
    create_shared_import_linker,
    create_sovereign_import_linker,
    Bytes,
    DelegatedExecution,
    EmptyJournalTrie,
    ExitCode,
    IJournaledTrie,
    SysFuncIdx::STATE,
    F254,
    POSEIDON_EMPTY,
    STATE_DEPLOY,
    STATE_MAIN,
};
use hashbrown::{hash_map::Entry, HashMap};
use rwasm::{
    core::{ImportLinker, Trap},
    engine::{bytecode::Instruction, DropKeep, RwasmConfig, StateRouterConfig},
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
    sync::atomic::{AtomicU64, Ordering},
};

pub type DefaultEmptyRuntimeDatabase = JournaledTrie<ZkTrieStateDb<InMemoryTrieDb>>;

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

pub struct RuntimeContext<DB: IJournaledTrie> {
    // context inputs
    pub(crate) bytecode: BytecodeOrHash,
    pub(crate) fuel_limit: u64,
    pub(crate) state: u32,
    #[deprecated(note = "this parameter can be removed, we filter on the AOT level")]
    pub(crate) is_shared: bool,
    pub(crate) input: Vec<u8>,
    pub(crate) context: Vec<u8>,
    pub(crate) depth: u32,
    // context outputs
    pub(crate) execution_result: ExecutionResult,
    // storage
    pub(crate) jzkt: Option<DB>,
}

impl<DB: IJournaledTrie> Debug for RuntimeContext<DB> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime context")
    }
}

impl<DB: IJournaledTrie> Default for RuntimeContext<DB> {
    fn default() -> Self {
        Self {
            bytecode: Default::default(),
            fuel_limit: 0,
            state: 0,
            is_shared: false,
            input: vec![],
            context: vec![],
            depth: 0,
            execution_result: Default::default(),
            jzkt: None,
        }
    }
}

impl<DB: IJournaledTrie> RuntimeContext<DB> {
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

    pub fn with_context(mut self, context: Vec<u8>) -> Self {
        self.context = context;
        self
    }

    pub fn change_input(&mut self, input_data: Vec<u8>) {
        self.input = input_data;
    }

    pub fn change_context(&mut self, new_context: Vec<u8>) {
        self.context = new_context;
    }

    pub fn with_state(mut self, state: u32) -> Self {
        self.state = state;
        self
    }

    pub fn with_is_shared(mut self, is_shared: bool) -> Self {
        self.is_shared = is_shared;
        self
    }

    pub fn with_fuel_limit(mut self, fuel_limit: u64) -> Self {
        self.fuel_limit = fuel_limit;
        self
    }

    pub fn with_jzkt(mut self, jzkt: DB) -> Self {
        self.jzkt = Some(jzkt);
        self
    }

    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    pub fn jzkt(&self) -> &DB {
        self.jzkt.as_ref().expect("jzkt is not initialized")
    }

    pub fn depth(&self) -> u32 {
        self.depth
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

    pub fn context(&self) -> &Vec<u8> {
        self.context.as_ref()
    }

    pub fn context_size(&self) -> u32 {
        self.context.len() as u32
    }

    pub fn argv_buffer(&self) -> Vec<u8> {
        self.input().clone()
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.execution_result.output
    }

    pub fn output_mut(&mut self) -> &mut Vec<u8> {
        &mut self.execution_result.output
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

    pub fn clean_output(&mut self) {
        self.execution_result.output = vec![];
    }
}

#[derive(Default, Clone)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub output: Vec<u8>,
    pub fuel_consumed: u64,
    pub return_data: Vec<u8>,
}

impl ExecutionResult {
    pub fn new_error(exit_code: i32) -> Self {
        Self {
            exit_code,
            ..Default::default()
        }
    }
}

static mut GLOBAL_RUNTIME_INDEX: AtomicU64 = AtomicU64::new(0);

struct RecoverableRuntime {
    runtime: Runtime<DefaultEmptyRuntimeDatabase>,
    resumable_invocation: ResumableInvocation,
}

impl RecoverableRuntime {
    fn state(&self) -> &SysExecResumable {
        self.resumable_invocation
            .host_error()
            .downcast_ref::<SysExecResumable>()
            .expect("can't downcast resumable invocation")
    }
}

pub struct CachingRuntime {
    // TODO(dmitry123): "add expiration to this map to avoid memory leak"
    modules: HashMap<F254, Module>,
    recoverable_runtimes: HashMap<u64, RecoverableRuntime>,
}

impl CachingRuntime {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            recoverable_runtimes: HashMap::new(),
        }
    }

    fn new_engine() -> Engine {
        // we can safely use sovereign import linker because all protected are filtered out during
        // a translation process
        let import_linker = Runtime::new_sovereign_linker();
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
            .consume_fuel(true);
        Engine::new(&config)
    }

    pub fn init_module(
        &mut self,
        engine: &Engine,
        rwasm_hash: F254,
        rwasm_bytecode: &[u8],
    ) -> Result<&Module, RuntimeError> {
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

pub struct Runtime<DB: IJournaledTrie> {
    // store and linker
    pub(crate) store: Store<RuntimeContext<DB>>,
    pub(crate) linker: Linker<RuntimeContext<DB>>,
}

impl Runtime<EmptyJournalTrie> {
    pub fn new_sovereign_linker() -> ImportLinker {
        create_sovereign_import_linker()
    }
    pub fn new_shared_linker() -> ImportLinker {
        create_shared_import_linker()
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
}

impl<DB: IJournaledTrie> Runtime<DB> {
    pub fn run_with_context(
        runtime_context: RuntimeContext<DB>,
    ) -> Result<ExecutionResult, RuntimeError> {
        Self::new(runtime_context).call()
    }

    pub fn new(mut runtime_context: RuntimeContext<DB>) -> Self {
        // make sure bytecode hash is resolved
        runtime_context.bytecode = runtime_context.bytecode.with_resolved_hash();

        // use existing engine or create a new one
        let engine = CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            let rwasm_hash = runtime_context.bytecode.resolve_hash();
            caching_runtime
                .resolve_module(&rwasm_hash)
                .map(|module| module.engine.clone())
                .unwrap_or_else(|| CachingRuntime::new_engine())
        });

        // create new linker and store (it shares the same engine resources)
        let mut store = Store::<RuntimeContext<DB>>::new(&engine, runtime_context);
        let mut linker = Linker::<RuntimeContext<DB>>::new(&engine);

        // add fuel if limit is specified
        if store.data().fuel_limit > 0 {
            store.add_fuel(store.data().fuel_limit).unwrap();
        }

        // register linker trampolines for external calls
        if store.data().is_shared {
            runtime_register_shared_handlers(&mut linker, &mut store)
        } else {
            runtime_register_sovereign_handlers(&mut linker, &mut store)
        }

        Self { store, linker }
    }

    fn get_instance(&mut self) -> Result<Instance, RuntimeError> {
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
                        caching_runtime.init_module(self.store.engine(), hash, &bytecode)
                    }
                }
                BytecodeOrHash::Hash(hash) => {
                    // if we have only hash, then try to load module or fail fast
                    match caching_runtime.resolve_module(hash) {
                        Some(module) => Ok(module),
                        None => {
                            let rwasm_bytecode = self
                                .store
                                .data_mut()
                                .jzkt
                                .as_ref()
                                .ok_or(RuntimeError::UnloadedModule(*hash))?
                                .preimage(hash);
                            caching_runtime.init_module(self.store.engine(), *hash, &rwasm_bytecode)
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

    pub fn call(&mut self) -> Result<ExecutionResult, RuntimeError> {
        let instance = self.get_instance()?;

        let mut next_result = instance
            .get_func(&mut self.store, "main")
            .ok_or(RuntimeError::MissingEntrypoint)?
            .call_resumable(&mut self.store, &[], &mut [])
            .map_err(Into::<RuntimeError>::into);
        loop {
            let resumable_invocation = match next_result {
                Ok(resumable) => match resumable {
                    ResumableCall::Finished => return self.handle_execution_result(None),
                    ResumableCall::Resumable(state) => state,
                },
                Err(err) => return self.handle_execution_result(Some(err)),
            };

            // if we have an exit code then return it, somehow execution failed, maybe if was out of
            // fuel, memory out of bounds or stack overflow/underflow
            if let Some(exit_code) = resumable_invocation.host_error().i32_exit_status() {
                return self.handle_execution_result(Some(RuntimeError::ExitCode(exit_code)));
            }

            // if we can't downcast our resumable invocation state, then something unexpected
            // happened, and we can only return an error,
            // but maybe the crash is safer
            if let Some(delayed_state) = resumable_invocation
                .host_error()
                .downcast_ref::<SysExecResumable>()
            {
                if delayed_state.depth_level > 0 {
                    return self.handle_resumable_state(resumable_invocation);
                }
                // if we're at zero depth level, then we can safely execute function
                // since this call is initiated on the root level and it is trusted
                let exit_code = SyscallExec::fn_continue(
                    Caller::new(&mut self.store, Some(&instance)),
                    delayed_state,
                )
                .unwrap_or_else(|exit_code| exit_code.into());
                let exit_code = Value::I32(exit_code.into_i32());
                next_result = resumable_invocation
                    .resume(self.store.as_context_mut(), &[exit_code], &mut [])
                    .map_err(Into::<RuntimeError>::into);
            } else {
                // TODO(dmitry123): maybe crash is safer?
                return Err(RuntimeError::Rwasm(
                    ExitCode::TransactError.into_trap().into(),
                ));
            };
        }
    }

    fn handle_resumable_state(
        &mut self,
        resumable_invocation: ResumableInvocation,
    ) -> Result<ExecutionResult, RuntimeError> {
        let delayed_state = resumable_invocation
            .host_error()
            .downcast_ref::<SysExecResumable>()
            .unwrap();
        // we disallow nested calls at non-root levels
        // so we must save the current state
        // to interrupt execution and delegate decision-making
        // to the root execution
        let return_data = self.store.data_mut().return_data_mut();
        assert!(return_data.is_empty(), "return data must be empty");
        // serialize delegated execution state,
        // but we don't serialize registers and stack state,
        // instead we remember it inside the internal structure
        // and assign a special identifier for recovery
        let encoded_state = delayed_state.delegated_execution.to_bytes();
        return_data.extend(encoded_state.as_ref());
        // save the current runtime state for future recovery
        CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            let call_id = unsafe { GLOBAL_RUNTIME_INDEX.fetch_add(1u64, Ordering::AcqRel) };
            // let recoverable_runtime = RecoverableRuntime {
            //     runtime: self,
            //     resumable_invocation,
            // };
            // caching_runtime
            //     .recoverable_runtimes
            //     .insert(call_id, recoverable_runtime);
        });
        // interruption is a special exit code that indicates to the root what happened inside
        // the call
        self.handle_execution_result(Some(RuntimeError::ExitCode(
            ExitCode::Interruption.into_i32(),
        )))
    }

    fn handle_execution_result(
        &self,
        err: Option<RuntimeError>,
    ) -> Result<ExecutionResult, RuntimeError> {
        let mut execution_result = self.store.data().execution_result.clone();
        execution_result.fuel_consumed = self.store.fuel_consumed().unwrap_or_default();
        if let Some(err) = err {
            execution_result.exit_code = Runtime::catch_trap(&err);
        }
        return Ok(execution_result);
    }

    pub fn store(&self) -> &Store<RuntimeContext<DB>> {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store<RuntimeContext<DB>> {
        &mut self.store
    }

    pub fn data(&self) -> &RuntimeContext<DB> {
        self.store.data()
    }

    pub fn data_mut(&mut self) -> &mut RuntimeContext<DB> {
        self.store.data_mut()
    }
}
