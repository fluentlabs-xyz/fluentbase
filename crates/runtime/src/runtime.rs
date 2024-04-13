use crate::types::InMemoryTrieDb;
use crate::zktrie::ZkTrieStateDb;
use crate::{
    instruction::{runtime_register_shared_handlers, runtime_register_sovereign_handlers},
    types::RuntimeError,
    JournaledTrie,
};
use fluentbase_types::{
    create_shared_import_linker, create_sovereign_import_linker, EmptyJournalTrie, ExitCode,
    IJournaledTrie,
};
use rwasm::{
    core::ImportLinker, rwasm::RwasmModule, Engine, FuelConsumptionMode, Instance, Linker, Module,
    Store,
};

pub type DefaultEmptyRuntimeDatabase = JournaledTrie<ZkTrieStateDb<InMemoryTrieDb>>;

pub struct RuntimeContext<DB: IJournaledTrie> {
    // context inputs
    pub(crate) bytecode: Vec<u8>,
    pub(crate) fuel_limit: u64,
    pub(crate) state: u32,
    pub(crate) is_shared: bool,
    pub(crate) catch_trap: bool,
    pub(crate) input: Vec<u8>,
    // context outputs
    pub(crate) execution_result: ExecutionResult,
    // storage
    pub(crate) jzkt: Option<DB>,
}

impl<DB: IJournaledTrie> Default for RuntimeContext<DB> {
    fn default() -> Self {
        Self {
            bytecode: Default::default(),
            fuel_limit: 0,
            state: 0,
            is_shared: false,
            catch_trap: true,
            input: vec![],
            execution_result: Default::default(),
            jzkt: None,
        }
    }
}

impl<DB: IJournaledTrie> RuntimeContext<DB> {
    pub fn new<I: Into<Vec<u8>>>(bytecode: I) -> Self {
        Self {
            bytecode: bytecode.into(),
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

    pub fn with_catch_trap(mut self, catch_trap: bool) -> Self {
        self.catch_trap = catch_trap;
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

    pub fn jzkt(&mut self) -> &DB {
        self.jzkt.as_ref().expect("jzkt is not initialized")
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

pub struct Runtime<DB: IJournaledTrie> {
    pub(crate) engine: Engine,
    pub(crate) module: Module,
    pub(crate) linker: Linker<RuntimeContext<DB>>,
    pub(crate) store: Store<RuntimeContext<DB>>,
    pub(crate) instance: Option<Instance>,
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
            _ => return ExitCode::UnknownError as i32,
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
        import_linker: ImportLinker,
    ) -> Result<ExecutionResult, RuntimeError> {
        let catch_error = runtime_context.catch_trap;
        let runtime = Self::new(runtime_context, import_linker);
        if catch_error && runtime.is_err() {
            return Ok(ExecutionResult::new_error(Runtime::catch_trap(
                runtime.err().as_ref().unwrap(),
            )));
        }
        let mut runtime = runtime?;
        runtime.store.data_mut().clean_output();
        runtime.call()
    }

    pub fn new(
        runtime_context: RuntimeContext<DB>,
        import_linker: ImportLinker,
    ) -> Result<Self, RuntimeError> {
        let mut result = Self::new_uninit(runtime_context, import_linker)?;
        result.register_bindings();
        result.instantiate()?;
        Ok(result)
    }

    pub fn new_uninit(
        runtime_context: RuntimeContext<DB>,
        import_linker: ImportLinker,
    ) -> Result<Self, RuntimeError> {
        let fuel_limit = runtime_context.fuel_limit;

        let engine = {
            let mut config = RwasmModule::default_config(Some(import_linker));
            config.floats(false);
            if fuel_limit > 0 {
                config.fuel_consumption_mode(FuelConsumptionMode::Eager);
                config.consume_fuel(true);
            }
            Engine::new(&config)
        };

        let module = {
            let reduced_module = RwasmModule::new(runtime_context.bytecode.as_ref())
                .map_err(Into::<RuntimeError>::into)?;
            let module_builder = reduced_module.to_module_builder(&engine);
            module_builder.finish()
        };

        let linker = Linker::<RuntimeContext<DB>>::new(&engine);
        let mut store = Store::<RuntimeContext<DB>>::new(&engine, runtime_context);

        if fuel_limit > 0 {
            store.add_fuel(fuel_limit).unwrap();
        }

        let result = Self {
            engine,
            module,
            linker,
            store,
            instance: None,
        };

        Ok(result)
    }

    pub fn instantiate(&mut self) -> Result<(), RuntimeError> {
        let instance = self
            .linker
            .instantiate(&mut self.store, &self.module)
            .map_err(Into::<RuntimeError>::into)?
            .start(&mut self.store)
            .map_err(Into::<RuntimeError>::into)?;
        self.instance = Some(instance);
        Ok(())
    }

    pub fn call(&mut self) -> Result<ExecutionResult, RuntimeError> {
        let func = self
            .instance
            .unwrap()
            .get_func(&mut self.store, "main")
            .ok_or(RuntimeError::MissingEntrypoint)?;
        let res = func
            .call(&mut self.store, &[], &mut [])
            .map_err(Into::<RuntimeError>::into);
        match res {
            Ok(_) => {}
            Err(err) => {
                let exit_code = Runtime::catch_trap(&err);
                if exit_code != 0 && !self.store.data().catch_trap {
                    return Err(err);
                }
                self.store.data_mut().execution_result.exit_code = exit_code;
            }
        }
        // we need to restore trace to recover missing opcode values
        let mut execution_result = self.store.data().execution_result.clone();
        execution_result.fuel_consumed = self.store.fuel_consumed().unwrap_or_default();
        Ok(execution_result)
    }

    pub fn register_bindings(&mut self) {
        if !self.store.data().is_shared {
            runtime_register_sovereign_handlers(&mut self.linker, &mut self.store)
        } else {
            runtime_register_shared_handlers(&mut self.linker, &mut self.store)
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
