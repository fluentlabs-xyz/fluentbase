use crate::instruction::sys_exec_hash::SysExecHash;
use crate::{
    instruction::{runtime_register_shared_handlers, runtime_register_sovereign_handlers},
    types::{InMemoryTrieDb, RuntimeError},
    zktrie::ZkTrieStateDb,
    JournaledTrie,
};
use fluentbase_poseidon::poseidon_hash;
use fluentbase_types::{
    create_shared_import_linker, create_sovereign_import_linker, Bytes, EmptyJournalTrie, ExitCode,
    IJournaledTrie, F254, POSEIDON_EMPTY,
};
use hashbrown::hash_map::Entry;
use hashbrown::HashMap;
use rwasm::core::{HostError, Trap};
use rwasm::{
    core::ImportLinker, rwasm::RwasmModule, AsContextMut, Engine, FuelConsumptionMode, Instance,
    Linker, Module, ResumableCall, Store, Value,
};
use std::cell::RefCell;
use std::fmt::{Debug, Display, Formatter};
use std::mem::take;

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

pub struct RuntimeContext<DB: IJournaledTrie> {
    // context inputs
    pub(crate) bytecode: BytecodeOrHash,
    pub(crate) fuel_limit: u64,
    pub(crate) state: u32,
    pub(crate) is_shared: bool,
    pub(crate) input: Vec<u8>,
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

    pub fn change_input(&mut self, input_data: Vec<u8>) {
        self.input = input_data;
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

    pub fn jzkt(&mut self) -> &DB {
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

    pub fn input_count(&self) -> u32 {
        self.input.len() as u32
    }

    pub fn input_size(&self) -> u32 {
        self.input.len() as u32
    }

    pub fn argv_buffer(&self) -> Vec<u8> {
        self.input().clone()
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.execution_result.output
    }

    pub fn return_data(&self) -> &Vec<u8> {
        &self.execution_result.return_data
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

pub struct CachingRuntime {
    // TODO(dmitry123): "add expiration to this map to avoid memory leak"
    modules: HashMap<F254, Module>,
}

impl CachingRuntime {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    fn new_engine() -> Engine {
        // we can safely use sovereign import linker because all protected are filtered out during translation process
        let import_linker = Runtime::new_sovereign_linker();
        let mut config = RwasmModule::default_config(Some(import_linker));
        config
            .floats(false)
            .fuel_consumption_mode(FuelConsumptionMode::Eager)
            .consume_fuel(true);
        Engine::new(&config)
    }

    pub fn init_module(
        &mut self,
        rwasm_hash: F254,
        rwasm_bytecode: &[u8],
    ) -> Result<&Module, RuntimeError> {
        let entry = match self.modules.entry(rwasm_hash) {
            Entry::Occupied(_) => return Err(RuntimeError::UnloadedModule(rwasm_hash)),
            Entry::Vacant(entry) => entry,
        };
        // empty bytecode we can't execute so just return Ok exit code
        if rwasm_bytecode.is_empty() {
            return Err(RuntimeError::Rwasm(ExitCode::Ok.into_trap().into()));
        }
        let reduced_module =
            RwasmModule::new(rwasm_bytecode).map_err(Into::<RuntimeError>::into)?;
        let engine = Self::new_engine();
        let module_builder = reduced_module.to_module_builder(&engine);
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
    pub(crate) store: Store<RuntimeContext<DB>>,
    pub(crate) instance: Instance,
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

#[derive(Debug)]
pub struct DelayedExecutionContext {
    pub bytecode_hash32: [u8; 32],
    pub input: Vec<u8>,
    pub return_len: u32,
    pub fuel_limit: u32,
    pub state: u32,
}

impl Display for DelayedExecutionContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime resume error")
    }
}

impl HostError for DelayedExecutionContext {}

impl<DB: IJournaledTrie> Runtime<DB> {
    pub fn run_with_context(
        runtime_context: RuntimeContext<DB>,
        import_linker: ImportLinker,
    ) -> Result<ExecutionResult, RuntimeError> {
        todo!("not implemented")
        // let runtime = Self::new(runtime_context, import_linker);
        // if runtime.is_err() {
        //     return Ok(ExecutionResult::new_error(Runtime::catch_trap(
        //         &runtime.err().as_ref().unwrap().1,
        //     )));
        // }
        // let mut runtime = runtime?;
        // runtime.store.data_mut().clean_output();
        // runtime.call()
    }

    pub fn new(
        mut runtime_context: RuntimeContext<DB>,
        import_linker: ImportLinker,
    ) -> Result<Self, RuntimeError> {
        let (store, instance) = CACHING_RUNTIME.with_borrow_mut(|caching_runtime| {
            let bytecode_repr = take(&mut runtime_context.bytecode);

            // resolve cached module or init it
            let module = match &bytecode_repr {
                BytecodeOrHash::Bytecode(bytecode, hash) => {
                    let hash = hash.unwrap_or_else(|| F254::from(poseidon_hash(&bytecode)));
                    // if we have cached module then use it, otherwise create new one and cache
                    if let Some(engine_module) = caching_runtime.resolve_module(&hash) {
                        Ok(engine_module)
                    } else {
                        caching_runtime.init_module(hash, &bytecode)
                    }
                }
                BytecodeOrHash::Hash(hash) => {
                    // if we have only hash then try to load module or fail fast
                    match caching_runtime.resolve_module(hash) {
                        Some(engine_module) => Ok(engine_module),
                        None => {
                            let rwasm_bytecode = runtime_context
                                .jzkt
                                .as_ref()
                                .ok_or(RuntimeError::UnloadedModule(*hash))?
                                .preimage(hash);
                            caching_runtime.init_module(*hash, &rwasm_bytecode)
                        }
                    }
                }
            }?;

            // create new linker and store (it shares same engine resources)
            let mut linker = Linker::<RuntimeContext<DB>>::new(&module.engine);
            let mut store = Store::<RuntimeContext<DB>>::new(&module.engine, runtime_context);

            // add fuel if limit is specified
            if store.data().fuel_limit > 0 {
                store.add_fuel(store.data().fuel_limit).unwrap();
            }

            // register linker trampolines for external calls
            if !store.data().is_shared {
                runtime_register_sovereign_handlers(&mut linker, &mut store)
            } else {
                runtime_register_shared_handlers(&mut linker, &mut store)
            }

            // init instance
            let instance = linker
                .instantiate(&mut store, &module)
                .map_err(Into::<RuntimeError>::into)?
                .start(&mut store)
                .map_err(Into::<RuntimeError>::into)?;

            Ok::<(Store<RuntimeContext<DB>>, Instance), RuntimeError>((store, instance))
        })?;

        Ok(Self { store, instance })
    }

    pub fn call(&mut self) -> Result<ExecutionResult, RuntimeError> {
        let mut next_result = self
            .instance
            .get_func(&mut self.store, "main")
            .ok_or(RuntimeError::MissingEntrypoint)?
            .call_resumable(&mut self.store, &[], &mut [])
            .map_err(Into::<RuntimeError>::into);
        loop {
            match next_result {
                Ok(resumable) => match resumable {
                    ResumableCall::Finished => {
                        let mut execution_result = self.store.data().execution_result.clone();
                        execution_result.fuel_consumed =
                            self.store.fuel_consumed().unwrap_or_default();
                        return Ok(execution_result);
                    }
                    ResumableCall::Resumable(state) => {
                        // check i32 exit code
                        let exit_code = if let Some(exit_code) =
                            state.host_error().i32_exit_status()
                        {
                            // if we have exit code then just return it, somehow execution failed, maybe if was out of fuel
                            let mut execution_result = self.store.data().execution_result.clone();
                            execution_result.exit_code = exit_code;
                            return Ok(execution_result);
                        } else if let Some(delayed_state) =
                            state.host_error().downcast_ref::<DelayedExecutionContext>()
                        {
                            // execute `_sys_exec_hash` function
                            match SysExecHash::fn_impl(
                                self.store.data_mut(),
                                &delayed_state.bytecode_hash32,
                                delayed_state.input.clone(),
                                delayed_state.return_len,
                                delayed_state.fuel_limit as u64,
                                delayed_state.state,
                            ) {
                                Ok(_consumed_fuel) => {
                                    // TODO(dmitry123): "write fuel consumed and return data into memory?"
                                    ExitCode::Ok.into_i32()
                                }
                                Err(exit_code) => exit_code,
                            }
                        } else {
                            return Err(RuntimeError::Rwasm(
                                Trap::i32_exit(ExitCode::TransactError.into_i32()).into(),
                            ));
                        };
                        // resume call with exit code
                        let exit_code = Value::I32(exit_code);
                        next_result = state
                            .resume(self.store.as_context_mut(), &[exit_code], &mut [])
                            .map_err(Into::<RuntimeError>::into);
                    }
                },
                Err(err) => {
                    let mut execution_result = self.store.data().execution_result.clone();
                    execution_result.fuel_consumed = self.store.fuel_consumed().unwrap_or_default();
                    execution_result.exit_code = Runtime::catch_trap(&err);
                    return Ok(execution_result);
                }
            }
        }
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
