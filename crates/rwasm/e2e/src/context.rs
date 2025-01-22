use super::{TestDescriptor, TestError, TestProfile, TestSpan};
use crate::handler::{
    TestingContext,
    TestingSyscallHandler,
    ENTRYPOINT_FUNC_IDX,
    FUNC_PRINT,
    FUNC_PRINT_F32,
    FUNC_PRINT_F64,
    FUNC_PRINT_I32,
    FUNC_PRINT_I32_F32,
    FUNC_PRINT_I64,
    FUNC_PRINT_I64_F64,
};
use anyhow::Result;
use fluentbase_rwasm::{
    AlwaysFailingSyscallHandler,
    Caller,
    RwasmError,
    RwasmExecutor,
    SimpleCallHandler,
};
use rwasm::{
    core::{ImportLinker, ValueType, F32, F64},
    engine::{bytecode::Instruction, RwasmConfig, StateRouterConfig},
    func::FuncIdx,
    module::{ImportName, Imported},
    rwasm::{BinaryFormat, BinaryFormatWriter, RwasmModule},
    Config,
    Engine,
    Extern,
    Func,
    FuncType,
    Global,
    Linker,
    Memory,
    MemoryType,
    Module,
    Mutability,
    Store,
    Table,
    TableType,
    Value,
};
use std::{cell::RefCell, collections::HashMap, hash::Hash, rc::Rc, sync::Arc};
use wast::token::{Id, Span};

type TestingRwasmExecutor = RwasmExecutor<TestingSyscallHandler, TestingContext>;
type Instance = Rc<RefCell<TestingRwasmExecutor>>;

/// The context of a single Wasm test spec suite run.
pub struct TestContext<'a> {
    /// The wasmi config
    config: Config,
    /// The `wasmi` engine used for executing functions used during the test.
    engine: Engine,
    /// The linker for linking together Wasm test modules.
    linker: Linker<()>,
    /// The store to hold all runtime data during the test.
    store: Store<()>,
    /// The list of all encountered Wasm modules belonging to the test.
    modules: Vec<RwasmModule>,
    /// The list of all instantiated modules.
    instances: HashMap<String, Instance>,
    import_linker: ImportLinker,
    extern_types: HashMap<String, FuncType>,
    extern_state: HashMap<String, u32>,
    /// The last touched module instance.
    last_instance: Option<Instance>,
    /// Profiling during the Wasm spec test run.
    profile: TestProfile,
    /// Intermediate results buffer that can be reused for calling Wasm functions.
    results: Vec<Value>,
    /// The descriptor of the test.
    ///
    /// Useful for printing better debug messages in case of failure.
    descriptor: &'a TestDescriptor,
}

impl<'a> TestContext<'a> {
    /// Creates a new [`TestContext`] with the given [`TestDescriptor`].
    pub fn new(descriptor: &'a TestDescriptor, config: Config) -> Self {
        let engine = Engine::new(&config);
        let mut linker = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let default_memory = Memory::new(&mut store, MemoryType::new(1, Some(2)).unwrap()).unwrap();
        let default_table = Table::new(
            &mut store,
            TableType::new(ValueType::FuncRef, 10, Some(20)),
            Value::default(ValueType::FuncRef),
        )
        .unwrap();

        let import_linker = ImportLinker::from([
            ("spectest", "print", FUNC_PRINT, 0),
            ("spectest", "print_i32", FUNC_PRINT_I32, 0),
            ("spectest", "print_i64", FUNC_PRINT_I64, 0),
            ("spectest", "print_f32", FUNC_PRINT_F32, 0),
            ("spectest", "print_f64", FUNC_PRINT_F64, 0),
            ("spectest", "print_i32_f32", FUNC_PRINT_I32_F32, 0),
            ("spectest", "print_f64_f64", FUNC_PRINT_I64_F64, 0),
        ]);

        TestContext {
            config,
            engine,
            linker,
            store,
            modules: Vec::new(),
            instances: HashMap::new(),
            import_linker,
            extern_types: Default::default(),
            extern_state: Default::default(),
            last_instance: None,
            profile: TestProfile::default(),
            results: Vec::new(),
            descriptor,
        }
    }
}

impl TestContext<'_> {
    /// Returns the file path of the associated `.wast` test file.
    fn test_path(&self) -> &str {
        self.descriptor.path()
    }

    /// Returns the [`TestDescriptor`] of the test context.
    pub fn spanned(&self, span: Span) -> TestSpan {
        self.descriptor.spanned(span)
    }

    /// Returns the [`Engine`] of the [`TestContext`].
    fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Returns a shared reference to the underlying [`Store`].
    pub fn store(&self) -> &Store<()> {
        &self.store
    }

    /// Returns an exclusive reference to the underlying [`Store`].
    pub fn store_mut(&mut self) -> &mut Store<()> {
        &mut self.store
    }

    /// Returns an exclusive reference to the test profile.
    pub fn profile(&mut self) -> &mut TestProfile {
        &mut self.profile
    }

    /// Compiles the Wasm module and stores it into the [`TestContext`].
    ///
    /// # Errors
    ///
    /// If creating the [`Module`] fails.
    pub fn compile_and_instantiate(
        &mut self,
        mut module: wast::core::Module,
    ) -> Result<(), TestError> {
        println!("\n --- creating module ---");
        // println!("{:?}", module);
        let module_name = module.id.map(|id| id.name());
        let wasm = module.encode().unwrap_or_else(|error| {
            panic!(
                "encountered unexpected failure to encode `.wast` module into `.wasm`:{}: {}",
                self.test_path(),
                error
            )
        });

        // extract all exports first to calculate rwasm config
        let rwasm_config = {
            let wasm_module = Module::new(self.engine(), &wasm[..])?;
            let mut states = Vec::<(String, u32)>::new();
            for (k, v) in wasm_module.exports.iter() {
                let func_idx = v.into_func_idx();
                if func_idx.is_none() {
                    continue;
                }
                let func_idx = func_idx.unwrap();
                let func_typ = wasm_module.get_export(k).unwrap();
                let func_typ = func_typ.func().unwrap();
                let state_value = 10000 + func_idx.into_u32();
                self.extern_types.insert(k.to_string(), func_typ.clone());
                self.extern_state.insert(k.to_string(), state_value);
                states.push((k.to_string(), state_value));
            }
            // println!("states={:?}", states);
            RwasmConfig {
                state_router: Some(StateRouterConfig {
                    states: states.into(),
                    opcode: Instruction::Call(u32::MAX.into()),
                }),
                entrypoint_name: None,
                import_linker: Some(self.import_linker.clone()),
                wrap_import_functions: true,
                translate_drop_keep: false,
            }
        };

        let mut config = Config::default();
        config
            .wasm_mutable_global(false)
            .wasm_saturating_float_to_int(false)
            .wasm_sign_extension(false)
            .wasm_multi_value(false)
            .wasm_mutable_global(true)
            .wasm_saturating_float_to_int(true)
            .wasm_sign_extension(true)
            .wasm_multi_value(true)
            .wasm_bulk_memory(true)
            .wasm_reference_types(true)
            .wasm_tail_call(true)
            .wasm_extended_const(true);
        config.rwasm_config(rwasm_config);

        let engine = Engine::new(&config);
        let wasm_module = Module::new(&engine, &wasm[..])?;

        let rwasm_module = RwasmModule::from_module(&wasm_module);
        // encode and decode rwasm module (to tests encoding/decoding flow)
        let mut encoded_rwasm_module = Vec::new();
        rwasm_module
            .write_binary_to_vec(&mut encoded_rwasm_module)
            .unwrap();
        let rwasm_module = RwasmModule::read_from_slice(&encoded_rwasm_module).unwrap();

        // println!();
        // #[allow(unused)]
        // fn trace_rwasm(rwasm_bytecode: &[u8]) {
        //     let rwasm_module = RwasmModule::new(rwasm_bytecode).unwrap();
        //     let mut func_length = 0usize;
        //     let mut expected_func_length = rwasm_module
        //         .func_section
        //         .first()
        //         .copied()
        //         .unwrap_or(u32::MAX) as usize;
        //     let mut func_index = 0usize;
        //     println!("\n -- function #{} -- ", func_index);
        //     for (i, instr) in rwasm_module.code_section.instr.iter().enumerate() {
        //         println!("{:02}: {:?}", i, instr);
        //         func_length += 1;
        //         if func_length == expected_func_length {
        //             func_index += 1;
        //             expected_func_length = rwasm_module
        //                 .func_section
        //                 .get(func_index)
        //                 .copied()
        //                 .unwrap_or(u32::MAX) as usize;
        //             if expected_func_length != u32::MAX as usize {
        //                 println!("\n -- function #{} -- ", func_index);
        //             }
        //             func_length = 0;
        //         }
        //     }
        //     println!("\n")
        // }
        // trace_rwasm(&encoded_rwasm_module);
        // println!();

        let mut executor =
            TestingRwasmExecutor::new(rwasm_module.instantiate(), None, TestingContext::default());
        executor.store_mut().context_mut().state = ENTRYPOINT_FUNC_IDX;
        println!(" --- entrypoint ---");
        let exit_code = executor.run().map_err(|err| {
            let trap_code = match err {
                RwasmError::TrapCode(trap_code) => trap_code,
                _ => unreachable!("not possible error: {:?}", err),
            };
            TestError::Wasmi(rwasm::Error::Trap(trap_code.into()))
        })?;
        assert_eq!(exit_code, 0);
        println!();

        let instance = Rc::new(RefCell::new(executor));

        if let Some(module_name) = module_name {
            self.instances
                .insert(module_name.to_string(), instance.clone());
        }
        self.last_instance = Some(instance);
        Ok(())
    }

    /// Loads the Wasm module instance with the given name.
    ///
    /// # Errors
    ///
    /// If there is no registered module instance with the given name.
    pub fn instance_by_name(&self, name: &str) -> Result<Instance, TestError> {
        self.instances
            .get(name)
            .cloned()
            .ok_or_else(|| TestError::InstanceNotRegistered {
                name: name.to_owned(),
            })
    }

    /// Loads the Wasm module instance with the given name or the last instantiated one.
    ///
    /// # Errors
    ///
    /// If there have been no Wasm module instances registered so far.
    pub fn instance_by_name_or_last(&self, name: Option<&str>) -> Result<Instance, TestError> {
        name.map(|name| self.instance_by_name(name))
            .unwrap_or_else(|| {
                self.last_instance
                    .clone()
                    .ok_or(TestError::NoModuleInstancesFound)
            })
    }

    /// Registers the given [`Instance`] with the given `name` and sets it as the last instance.
    pub fn register_instance(&mut self, name: &str, instance: Instance) {
        if self.instances.get(name).is_some() {
            // Already registered the instance.
            return;
        }
        self.instances.insert(name.to_string(), instance.clone());
        // for export in instance.exports(&self.store) {
        //     self.linker
        //         .define(name, export.name(), export.clone().into_extern())
        //         .unwrap_or_else(|error| {
        //             let field_name = export.name();
        //             let export = export.clone().into_extern();
        //             panic!(
        //                 "failed to define export {name}::{field_name}: \
        //                 {export:?}: {error}",
        //             )
        //         });
        // }
        self.last_instance = Some(instance);
    }

    /// Invokes the [`Func`] identified by `func_name` in [`Instance`] identified by `module_name`.
    ///
    /// If no [`Instance`] under `module_name` is found then invoke [`Func`] on the last
    /// instantiated [`Instance`].
    ///
    /// # Note
    ///
    /// Returns the results of the function invocation.
    ///
    /// # Errors
    ///
    /// - If no module instances can be found.
    /// - If no function identified with `func_name` can be found.
    /// - If function invokation returned an error.
    pub fn invoke(
        &mut self,
        module_name: Option<&str>,
        func_name: &str,
        args: &[Value],
    ) -> Result<&[Value], TestError> {
        println!("\n --- {} ---", func_name);

        let mut instance = self.instance_by_name_or_last(module_name)?;
        let mut instance = instance.borrow_mut();

        // We reset an instruction pointer to the state function position to re-invoke the function.
        // However, with different states.
        // Some tests might fail, and we might keep outdated signature value in the state,
        // make sure the state is clear before every new call.
        let pc = instance.store().context().program_counter as usize;
        instance.store_mut().reset(Some(pc));

        let func_state = self
            .extern_state
            .get(&func_name.to_string())
            .unwrap()
            .clone();

        let mut caller = Caller::new(instance.store_mut());
        for value in args {
            caller.stack_push(value.clone());
        }

        // change function state for router
        instance.store_mut().context_mut().state = func_state;
        let exit_code = instance.run().map_err(|err| {
            let trap_code = match err {
                RwasmError::TrapCode(trap_code) => trap_code,
                _ => unreachable!("not possible error: {:?}", err),
            };
            TestError::Wasmi(rwasm::Error::Trap(trap_code.into()))
        })?;
        // copy results
        let func_type = self.extern_types.get(func_name).unwrap();
        let len_results = func_type.results().len();
        self.results.clear();
        self.results.resize(len_results, Value::I32(0));
        let mut caller = Caller::new(instance.store_mut());
        for (i, val_type) in func_type.results().iter().rev().enumerate() {
            let popped_value = caller.stack_pop();
            self.results[len_results - 1 - i] = match val_type {
                ValueType::I32 => Value::I32(popped_value.into()),
                ValueType::I64 => Value::I64(popped_value.into()),
                ValueType::F32 => Value::F32(popped_value.into()),
                ValueType::F64 => Value::F64(popped_value.into()),
                _ => unreachable!("unsupported result type: {:?}", val_type),
            };
        }
        Ok(&self.results)
    }

    /// Returns the current value of the [`Global`] identifier by the given `module_name` and
    /// `global_name`.
    ///
    /// # Errors
    ///
    /// - If no module instances can be found.
    /// - If no global variable identifier with `global_name` can be found.
    pub fn get_global(
        &self,
        module_name: Option<Id>,
        global_name: &str,
    ) -> Result<Value, TestError> {
        let module_name = module_name.map(|id| id.name());
        // let instance = self.instance_by_name_or_last(module_name)?;
        // let global = instance
        //     .get_export(&self.store, global_name)
        //     .and_then(Extern::into_global)
        //     .ok_or_else(|| TestError::GlobalNotFound {
        //         module_name: module_name.map(|name| name.to_string()),
        //         global_name: global_name.to_string(),
        //     })?;
        // let value = global.get(&self.store);
        // Ok(value)
        todo!()
    }
}
