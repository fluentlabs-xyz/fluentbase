use crate::{
    instruction::{
        runtime_register_shared_handlers,
        runtime_register_shared_linkers,
        runtime_register_sovereign_handlers,
        runtime_register_sovereign_linkers,
    },
    storage::TrieStorage,
    types::RuntimeError,
};
use fluentbase_types::{
    AccountDb,
    Address,
    ExitCode,
    PreimageDb,
    RECURSIVE_MAX_DEPTH,
    STACK_MAX_HEIGHT,
};
use rwasm_codegen::{
    rwasm::{
        engine::Tracer,
        AsContextMut,
        Config,
        Engine,
        FuelConsumptionMode,
        Func,
        FuncType,
        Instance,
        IntoFunc,
        Linker,
        Module,
        StackLimits,
        Store,
    },
    ImportLinker,
    InstructionSet,
    ReducedModule,
    ReducedModuleError,
};
use std::{cell::RefCell, mem::take, rc::Rc};

pub struct RuntimeContext<'t, T> {
    pub context: Option<&'t mut T>,
    // context inputs
    pub(crate) bytecode: Vec<u8>,
    pub(crate) fuel_limit: u32,
    pub(crate) state: u32,
    pub(crate) is_shared: bool,
    pub(crate) catch_trap: bool,
    pub(crate) input: Vec<u8>,
    pub(crate) is_static: bool,
    pub(crate) caller: Address,
    pub(crate) address: Address,
    pub(crate) func_type: Option<FuncType>,
    // context outputs
    pub(crate) exit_code: i32,
    pub(crate) output: Vec<u8>,
    pub(crate) consumed_fuel: u32,
    pub(crate) return_data: Vec<u8>,
    // storage
    pub(crate) account_db: Option<Rc<RefCell<dyn AccountDb>>>,
    pub(crate) trie_db: Option<Rc<RefCell<dyn TrieStorage>>>,
    pub(crate) preimage_db: Option<Rc<RefCell<dyn PreimageDb>>>,
}

impl<'ctx, CTX> Clone for RuntimeContext<'ctx, CTX> {
    fn clone(&self) -> Self {
        Self {
            context: None,
            func_type: None,
            bytecode: self.bytecode.clone(),
            fuel_limit: self.fuel_limit.clone(),
            state: self.state.clone(),
            is_shared: self.is_shared.clone(),
            catch_trap: self.catch_trap.clone(),
            input: self.input.clone(),
            is_static: self.is_static.clone(),
            caller: self.caller.clone(),
            address: self.address.clone(),
            exit_code: self.exit_code.clone(),
            output: self.output.clone(),
            consumed_fuel: self.consumed_fuel.clone(),
            return_data: self.return_data.clone(),
            account_db: self.account_db.clone(),
            trie_db: self.trie_db.clone(),
            preimage_db: self.preimage_db.clone(),
        }
    }
}

impl<'t, T> Default for RuntimeContext<'t, T> {
    fn default() -> Self {
        Self {
            context: None,
            func_type: None,
            bytecode: vec![],
            fuel_limit: 0,
            state: 0,
            is_shared: false,
            catch_trap: true,
            input: vec![],
            is_static: false,
            caller: Default::default(),
            address: Default::default(),
            exit_code: 0,
            output: vec![],
            consumed_fuel: 0,
            return_data: vec![],
            account_db: None,
            trie_db: None,
            preimage_db: None,
        }
    }
}

impl<'t, T> RuntimeContext<'t, T> {
    pub fn new<I: Into<Vec<u8>>>(bytecode: I) -> Self {
        Self {
            bytecode: bytecode.into(),
            ..Default::default()
        }
    }

    pub fn with_func_type(mut self, func_type: FuncType) -> Self {
        self.func_type = Some(func_type);
        self
    }

    pub fn with_context(mut self, context: &'t mut T) -> Self {
        self.context = Some(context);
        self
    }

    pub fn with_input(mut self, input_data: Vec<u8>) -> Self {
        self.input = input_data;
        self
    }

    pub fn with_is_static(mut self, is_static: bool) -> Self {
        self.is_static = is_static;
        self
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

    pub fn with_fuel_limit(mut self, fuel_limit: u32) -> Self {
        self.fuel_limit = fuel_limit;
        self
    }

    pub fn with_caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    pub fn with_address(mut self, address: Address) -> Self {
        self.address = address;
        self
    }

    pub fn with_account_db(mut self, account: Rc<RefCell<dyn AccountDb>>) -> Self {
        self.account_db = Some(account);
        self
    }

    pub fn with_trie_db(mut self, zktrie_db: Rc<RefCell<dyn TrieStorage>>) -> Self {
        self.trie_db = Some(zktrie_db);
        self
    }

    pub fn with_preimage_db(mut self, preimage_db: Rc<RefCell<dyn PreimageDb>>) -> Self {
        self.preimage_db = Some(preimage_db);
        self
    }

    pub fn take_context<F>(&mut self, func: F)
    where
        F: FnOnce(&&'t mut T),
    {
        if let Some(context) = &self.context {
            func(context)
        }
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
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
        &self.output
    }

    pub fn state(&self) -> u32 {
        self.state
    }

    pub fn clean_output(&mut self) {
        self.output = vec![];
    }
}

pub struct ExecutionResult<'t, T> {
    runtime_context: RuntimeContext<'t, T>,
    tracer: Tracer,
    fuel_consumed: Option<u64>,
}

impl<'t, T> ExecutionResult<'t, T> {
    pub fn cloned(store: &Store<RuntimeContext<'t, T>>) -> Self {
        Self {
            runtime_context: store.data().clone(),
            tracer: store.tracer().clone(),
            fuel_consumed: store.fuel_consumed(),
        }
    }

    pub fn taken(store: &mut Store<RuntimeContext<'t, T>>) -> Self {
        let fuel_consumed = store.fuel_consumed();
        Self {
            runtime_context: take(store.data_mut()),
            tracer: take(store.tracer_mut()),
            fuel_consumed,
        }
    }

    pub fn bytecode(&self) -> &Vec<u8> {
        &self.runtime_context.bytecode
    }

    pub fn tracer(&self) -> &Tracer {
        &self.tracer
    }

    pub fn data(&self) -> &RuntimeContext<'t, T> {
        &self.runtime_context
    }

    pub fn fuel_consumed(&self) -> Option<u64> {
        self.fuel_consumed
    }
}

#[allow(dead_code)]
pub struct Runtime<'t, T> {
    engine: Engine,
    bytecode: InstructionSet,
    module: Module,
    linker: Linker<RuntimeContext<'t, T>>,
    store: Store<RuntimeContext<'t, T>>,
    instance: Option<Instance>,
}

impl<'t, T> Runtime<'t, T> {
    pub fn new_sovereign_linker() -> ImportLinker {
        let mut import_linker = ImportLinker::default();
        runtime_register_sovereign_linkers::<T>(&mut import_linker);
        import_linker
    }

    pub fn new_shared_linker() -> ImportLinker {
        let mut import_linker = ImportLinker::default();
        runtime_register_shared_linkers::<T>(&mut import_linker);
        import_linker
    }

    pub fn run_with_context(
        mut runtime_context: RuntimeContext<'t, T>,
        import_linker: &ImportLinker,
    ) -> Result<ExecutionResult<'t, T>, RuntimeError> {
        let catch_error = runtime_context.catch_trap;
        let runtime = Self::new(runtime_context.clone(), import_linker);
        if catch_error && runtime.is_err() {
            runtime_context.exit_code = Self::catch_trap(runtime.err().unwrap());
            Ok(ExecutionResult {
                runtime_context,
                tracer: Default::default(),
                fuel_consumed: None,
            })
        } else {
            let mut runtime = runtime?;
            runtime.data_mut().clean_output();
            runtime.call()
        }
    }

    pub fn new(
        runtime_context: RuntimeContext<'t, T>,
        import_linker: &ImportLinker,
    ) -> Result<Self, RuntimeError> {
        let mut result = Self::new_uninit(runtime_context, import_linker)?;
        result.register_bindings();
        result.instantiate()?;
        Ok(result)
    }

    pub fn new_uninit(
        runtime_context: RuntimeContext<'t, T>,
        import_linker: &ImportLinker,
    ) -> Result<Self, RuntimeError> {
        let fuel_limit = runtime_context.fuel_limit;

        let engine = {
            let mut config = Config::default();
            config.set_stack_limits(
                StackLimits::new(STACK_MAX_HEIGHT, STACK_MAX_HEIGHT, RECURSIVE_MAX_DEPTH).unwrap(),
            );
            config.floats(false);
            if fuel_limit > 0 {
                config.fuel_consumption_mode(FuelConsumptionMode::Eager);
                config.consume_fuel(true);
            }
            Engine::new(&config)
        };

        let (module, bytecode) = {
            let reduced_module = ReducedModule::new(runtime_context.bytecode.as_slice())
                .map_err(Into::<RuntimeError>::into)?;
            let func_type = runtime_context
                .func_type
                .clone()
                .unwrap_or(FuncType::new([], []));
            let module_builder =
                reduced_module.to_module_builder(&engine, import_linker, func_type);
            (module_builder.finish(), reduced_module.bytecode().clone())
        };

        let linker = Linker::<RuntimeContext<T>>::new(&engine);
        let mut store = Store::<RuntimeContext<T>>::new(&engine, runtime_context);

        if fuel_limit > 0 {
            store.add_fuel(fuel_limit as u64).unwrap();
        }

        let result = Self {
            engine,
            bytecode,
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

    pub fn call(&mut self) -> Result<ExecutionResult<'t, T>, RuntimeError> {
        let func = self
            .instance
            .unwrap()
            .get_func(&mut self.store, "main")
            .ok_or(RuntimeError::ReducedModule(
                ReducedModuleError::MissingEntrypoint,
            ))?;
        let res = func
            .call(&mut self.store, &[], &mut [])
            .map_err(Into::<RuntimeError>::into);
        if self.store.data().catch_trap && res.is_err() {
            self.store.data_mut().exit_code = Self::catch_trap(res.err().unwrap());
        } else {
            res?;
        }
        // we need to restore trace to recover missing opcode values
        self.restore_trace();
        let execution_result = ExecutionResult::cloned(&self.store);
        Ok(execution_result)
    }

    pub fn add_binding<Params, Results>(
        &mut self,
        module: &'static str,
        name: &'static str,
        func: impl IntoFunc<RuntimeContext<'t, T>, Params, Results>,
    ) {
        self.linker
            .define(
                module,
                name,
                Func::wrap::<RuntimeContext<'t, T>, Params, Results>(
                    self.store.as_context_mut(),
                    func,
                ),
            )
            .unwrap();
    }

    pub fn register_bindings(&mut self) {
        if !self.data().is_shared {
            runtime_register_sovereign_handlers(&mut self.linker, &mut self.store)
        } else {
            runtime_register_shared_handlers(&mut self.linker, &mut self.store)
        }
    }

    pub fn catch_trap(err: RuntimeError) -> i32 {
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
        // otherwise its just an unknown error
        ExitCode::UnknownError as i32
    }

    fn restore_trace(&mut self) {
        // we need to fix logs, because we lost information about instr meta during conversion
        let tracer = self.store.tracer_mut();
        let call_id = tracer.logs.first().map(|v| v.call_id).unwrap_or_default();
        for log in tracer.logs.iter_mut() {
            if log.call_id != call_id {
                continue;
            }
            let instr = self.bytecode.get(log.index).unwrap();
            log.opcode = *instr;
        }
    }

    pub fn data(&self) -> &RuntimeContext<'t, T> {
        self.store.data()
    }

    pub fn data_mut(&mut self) -> &mut RuntimeContext<'t, T> {
        self.store.data_mut()
    }
}
