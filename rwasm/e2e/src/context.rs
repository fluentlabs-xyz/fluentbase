use super::{TestDescriptor, TestError, TestProfile, TestSpan};
use anyhow::Result;
use fluentbase_rwasm::{
    common::{Trap, UntypedValue, ValueType, F32, F64},
    engine::bytecode::Instruction,
    instruction_set,
    rwasm::{
        Compiler,
        CompilerConfig,
        DefaultImportHandler,
        FuncOrExport,
        ImportFunc,
        ImportLinker,
        ReducedModule,
    },
    value::WithType,
    AsContext,
    Caller,
    Config,
    Engine,
    Extern,
    ExternType,
    Func,
    FuncType,
    GlobalType,
    Instance,
    Linker,
    Memory,
    MemoryType,
    Module,
    Store,
    Table,
    TableType,
    Value,
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use wast::token::{Id, Span};

/// The context of a single Wasm test spec suite run.
#[derive(Debug)]
pub struct TestContext<'a> {
    /// The `runtime` engine used for executing functions used during the test.
    engine: Engine,
    /// The linker for linking together Wasm test modules.
    linker: Linker<DefaultImportHandler>,
    /// The store to hold all runtime data during the test.
    store: Store<DefaultImportHandler>,
    /// The list of all encountered Wasm modules belonging to the test.
    modules: Vec<Module>,
    /// The list of all instantiated modules.
    instances: HashMap<String, Instance>,

    last_exports: Option<Vec<String>>,

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

    binaries: HashMap<String, Vec<u8>>,

    main_router: HashMap<String, MainFunction>,

    func_type_check_idx: Rc<RefCell<Vec<FuncType>>>,
}

#[derive(Debug)]
struct MainFunction {
    pub fn_index: u32,
    pub fn_type: FuncType,
}

const SYS_STATE: u32 = 0xA002;
const SYS_INPUT: u32 = 0xF001;
const SYS_OUTPUT: u32 = 0xF002;
const SYS_INPUT_LEN: u32 = 0xF003;
const SYS_OUTPUT_LEN: u32 = 0xF004;

const SYS_PRINT_I32: u32 = 0xF005;
const SYS_PRINT_I64: u32 = 0xF006;
const SYS_PRINT_F32: u32 = 0xF007;
const SYS_PRINT_F64: u32 = 0xF008;
const SYS_PRINT_I32_F32: u32 = 0xF009;
const SYS_PRINT_F64_F64: u32 = 0xF010;
const SYS_PRINT: u32 = 0xF011;
const GLOBAL_START_INDEX: u32 = 0xF100;

impl<'a> TestContext<'a> {
    /// Creates a new [`TestContext`] with the given [`TestDescriptor`].
    pub fn new(descriptor: &'a TestDescriptor, config: Config) -> Self {
        let engine = Engine::new(&config);
        let mut linker = Linker::<DefaultImportHandler>::new(&engine);
        let mut store = Store::new(&engine, DefaultImportHandler::default());
        let default_memory = Memory::new(&mut store, MemoryType::new(1, Some(2)).unwrap()).unwrap();
        let default_table = Table::new(
            &mut store,
            TableType::new(ValueType::FuncRef, 10, Some(20)),
            Value::default(ValueType::FuncRef),
        )
        .unwrap();
        let global_i32 = Func::wrap(&mut store, || -> i32 { 666 });
        let global_i64 = Func::wrap(&mut store, || -> i64 { 666 });
        let global_f32 = Func::wrap(&mut store, || -> F32 { F32::from(666) });
        let global_f64 = Func::wrap(&mut store, || -> F64 { F64::from(666) });
        let print = Func::wrap(&mut store, || {
            println!("print");
        });
        let print_i32 = Func::wrap(&mut store, |value: i32| {
            println!("print_i32: {value}");
        });
        let print_i64 = Func::wrap(&mut store, |value: i64| {
            println!("print_i64: {value}");
        });
        let print_f32 = Func::wrap(&mut store, |value: F32| {
            println!("print_f32: {value:?}");
        });
        let print_f64 = Func::wrap(&mut store, |value: F64| {
            println!("print_f64: {value:?}");
        });
        let print_i32_f32 = Func::wrap(&mut store, |v0: i32, v1: F32| {
            println!("print_i32_f32: {v0:?} {v1:?}");
        });
        let print_f64_f64 = Func::wrap(&mut store, |v0: F64, v1: F64| {
            println!("print_f64_f64: {v0:?} {v1:?}");
        });

        let sys_state = Func::wrap(
            &mut store,
            |caller: Caller<'_, DefaultImportHandler>| -> Result<u32, Trap> {
                Ok(caller.data().state)
            },
        );

        let sys_input = Func::wrap(
            &mut store,
            |mut caller: Caller<'_, DefaultImportHandler>| -> Result<u64, Trap> {
                caller
                    .data_mut()
                    .next_input()
                    .map(|i| i.as_u64())
                    .ok_or(Trap::new("Input vector is empty"))
            },
        );

        let sys_output = Func::wrap(
            &mut store,
            |mut caller: Caller<'_, DefaultImportHandler>, output: i64| -> Result<(), Trap> {
                caller.data_mut().add_result(UntypedValue::from(output));
                Ok(())
            },
        );

        let sys_input_len = Func::wrap(
            &mut store,
            |caller: Caller<'_, DefaultImportHandler>| -> Result<u32, Trap> {
                Ok(caller.data().input.len() as u32)
            },
        );

        let sys_output_len = Func::wrap(
            &mut store,
            |caller: Caller<'_, DefaultImportHandler>| -> Result<u32, Trap> {
                Ok(caller.data().output_len() - caller.data().output().len() as u32)
            },
        );

        linker.define("spectest", "memory", default_memory).unwrap();
        linker.define("spectest", "table", default_table).unwrap();
        linker.define("spectest", "global_i32", global_i32).unwrap();
        linker.define("spectest", "global_i64", global_i64).unwrap();
        linker.define("spectest", "global_f32", global_f32).unwrap();
        linker.define("spectest", "global_f64", global_f64).unwrap();
        linker.define("spectest", "print", print).unwrap();
        linker.define("spectest", "print_i32", print_i32).unwrap();
        linker.define("spectest", "print_i64", print_i64).unwrap();
        linker.define("spectest", "print_f32", print_f32).unwrap();
        linker.define("spectest", "print_f64", print_f64).unwrap();
        linker
            .define("spectest", "print_i32_f32", print_i32_f32)
            .unwrap();
        linker
            .define("spectest", "print_f64_f64", print_f64_f64)
            .unwrap();
        linker.define("env", "_sys_state", sys_state).unwrap();

        linker.define("spectest", "_sys_input", sys_input).unwrap();
        linker
            .define("spectest", "_sys_output", sys_output)
            .unwrap();
        linker
            .define("spectest", "_sys_input_len", sys_input_len)
            .unwrap();
        linker
            .define("spectest", "_sys_output_len", sys_output_len)
            .unwrap();

        TestContext {
            engine,
            linker,
            store,
            modules: Vec::new(),
            instances: HashMap::new(),
            last_exports: None,
            last_instance: None,
            profile: TestProfile::default(),
            results: Vec::new(),
            descriptor,
            binaries: HashMap::new(),
            main_router: Default::default(),
            func_type_check_idx: Rc::new(RefCell::new(vec![])),
        }
    }

    pub fn set_state_by_name(
        &mut self,
        func_name: &str,
        module: Option<Id>,
    ) -> Result<(), TestError> {
        let inport_name = match module {
            Some(module) => format!("{}.{}", module.name(), func_name),
            None => func_name.to_string(),
        };
        let state = self
            .main_router
            .get(&inport_name)
            .or_else(|| self.main_router.get(func_name))
            .ok_or(TestError::MainFunctionNotFound)?;
        self.store.data_mut().state = state.fn_index;

        Ok(())
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
    pub fn store(&self) -> &Store<DefaultImportHandler> {
        &self.store
    }

    /// Returns an exclusive reference to the underlying [`Store`].
    pub fn store_mut(&mut self) -> &mut Store<DefaultImportHandler> {
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
        let wasm_binary = module.encode().unwrap_or_else(|error| {
            panic!(
                "encountered unexpected failure to encode `.wast` module into `.wasm`:{}: {}",
                self.test_path(),
                error
            )
        });
        let mut config = Config::default();
        config.consume_fuel(false);
        let engine = Engine::new(&config);
        let module2 = Module::new(&engine, wasm_binary.as_slice())?;
        for elem in module2.exports() {
            let instance = self.compile_and_instantiate_method(&wasm_binary, elem.name())?;
            self.binaries
                .insert(elem.name().to_string(), wasm_binary.clone());
            self.instances.insert(elem.name().to_string(), instance);
            self.last_instance = Some(instance);
        }
        Ok(())
    }

    pub fn compile_and_instantiate_with_router(
        &mut self,
        mut module: wast::core::Module,
    ) -> Result<(), TestError> {
        let wasm_binary = module.encode().unwrap_or_else(|error| {
            panic!(
                "encountered unexpected failure to encode `.wast` module into `.wasm`:{}: {}",
                self.test_path(),
                error
            )
        });
        let mut config = Config::default();
        config.wasm_tail_call(true);
        let name = module
            .name
            .map(|name| name.name)
            .or(module.id.map(|id| id.name()));
        config.consume_fuel(false);

        let module = Module::new(&self.engine, wasm_binary.as_slice())?;

        let mut import_linker = ImportLinker::default();
        let mut import_index = 0xF101;
        let mut global_index = 0;
        for import in module.imports() {
            match import.ty() {
                ExternType::Func(func_type) if import.module().ne("spectest") => {
                    import_linker.insert_function(ImportFunc::new_env(
                        import.module().to_string(),
                        import.name().to_string(),
                        import_index,
                        &func_type.params(),
                        &func_type.results(),
                        1,
                    ));
                    import_index += 1;
                    let exports = self
                        .instances
                        .get(format!("{}:{}", import.module(), import.name()).as_str())
                        .ok_or(TestError::NoModuleInstancesFound)?
                        .exports(&self.store)
                        .collect::<Vec<_>>();
                    if self
                        .linker
                        .get(self.store.as_context(), import.module(), import.name())
                        .is_none()
                    {
                        self.linker.define(
                            import.module(),
                            import.name(),
                            exports[0].clone().into_func().unwrap(),
                        )?;
                    }
                }
                ExternType::Global(global_type) => {
                    import_linker.insert_function(ImportFunc::new_env(
                        import.module().to_string(),
                        import.name().to_string(),
                        GLOBAL_START_INDEX as u16 + global_index,
                        &[],
                        &[global_type.content()],
                        1,
                    ));
                    global_index += 1;

                    if import.module().eq("spectest") {
                        continue;
                    }

                    let exports = self
                        .instances
                        .get(format!("{}:{}", import.module(), import.name()).as_str())
                        .ok_or(TestError::NoModuleInstancesFound)?
                        .exports(&self.store)
                        .collect::<Vec<_>>();
                    if self
                        .linker
                        .get(self.store.as_context(), import.module(), import.name())
                        .is_none()
                    {
                        self.linker.define(
                            import.module(),
                            import.name(),
                            exports[0].clone().into_func().unwrap(),
                        )?;
                    }
                }
                _ => {}
            }
        }

        let mut exports_names = None;

        let start_fn = module.get_start_fn();
        let mut router_index = 0;

        enum ExportRouter {
            Func(FuncType),
            Global((Instruction, GlobalType)),
        }

        let mut exports = module
            .exports()
            .filter_map(|export_type| match export_type.ty().clone() {
                ExternType::Func(func) => {
                    Some((export_type.name().to_string(), ExportRouter::Func(func)))
                }
                ExternType::Global(global) => {
                    let instruction = module.get_global_init(export_type.index()).unwrap();
                    Some((
                        export_type.name().to_string(),
                        ExportRouter::Global((instruction, global)),
                    ))
                }
                _ => None,
            })
            .map(|(name, export_router)| match export_router {
                ExportRouter::Func(func_type) => {
                    self.main_router.insert(
                        name.clone(),
                        MainFunction {
                            fn_index: router_index,
                            fn_type: func_type,
                        },
                    );
                    router_index += 1;
                    match exports_names.as_mut() {
                        None => {
                            exports_names = Some(vec![name.clone()]);
                        }
                        Some(names) => {
                            names.push(name.clone());
                        }
                    }
                    self.binaries.insert(name.clone(), wasm_binary.clone());

                    let static_name: &'static str = Box::leak(name.into_boxed_str());
                    FuncOrExport::Export(static_name)
                }
                ExportRouter::Global((instruction, global_type)) => {
                    self.main_router.insert(
                        name.clone(),
                        MainFunction {
                            fn_index: router_index,
                            fn_type: FuncType::new([], [global_type.content()]),
                        },
                    );
                    router_index += 1;
                    match exports_names.as_mut() {
                        None => {
                            exports_names = Some(vec![name.clone()]);
                        }
                        Some(names) => {
                            names.push(name.clone());
                        }
                    }
                    self.binaries.insert(name.clone(), wasm_binary.clone());

                    FuncOrExport::Global(instruction)
                }
            })
            .collect::<Vec<_>>();

        self.last_exports = exports_names;

        if let Some(start_fn) = start_fn {
            exports.push(FuncOrExport::Func(start_fn.into_u32()));
            self.main_router.insert(
                "main".to_string(),
                MainFunction {
                    fn_index: router_index,
                    fn_type: FuncType::new(vec![], vec![]),
                },
            );
        }

        #[cfg(feature = "e2e")]
        {
            import_linker.insert_function(ImportFunc::new_env(
                "env".to_string(),
                "_sys_state".to_string(),
                SYS_STATE as u16,
                &[],
                &[ValueType::I32],
                1,
            ));
            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "_sys_input".to_string(),
                SYS_INPUT as u16,
                &[],
                &[ValueType::I64],
                1,
            ));
            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "_sys_output".to_string(),
                SYS_OUTPUT as u16,
                &[ValueType::I64],
                &[],
                1,
            ));
            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "_sys_input_len".to_string(),
                SYS_INPUT_LEN as u16,
                &[],
                &[ValueType::I32],
                1,
            ));
            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "_sys_output_len".to_string(),
                SYS_OUTPUT_LEN as u16,
                &[],
                &[ValueType::I32],
                1,
            ));

            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "print_i32".to_string(),
                SYS_PRINT_I32 as u16,
                &[ValueType::I32],
                &[],
                1,
            ));

            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "print_i64".to_string(),
                SYS_PRINT_I64 as u16,
                &[ValueType::I64],
                &[],
                1,
            ));

            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "print_f32".to_string(),
                SYS_PRINT_F32 as u16,
                &[ValueType::F32],
                &[],
                1,
            ));

            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "print_f64".to_string(),
                SYS_PRINT_F64 as u16,
                &[ValueType::F64],
                &[],
                1,
            ));

            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "print_i32_f32".to_string(),
                SYS_PRINT_I32_F32 as u16,
                &[ValueType::I32, ValueType::F32],
                &[],
                1,
            ));

            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "print_f64_f64".to_string(),
                SYS_PRINT_F64_F64 as u16,
                &[ValueType::F64, ValueType::F64],
                &[],
                1,
            ));

            import_linker.insert_function(ImportFunc::new_env(
                "spectest".to_string(),
                "print".to_string(),
                SYS_PRINT as u16,
                &[],
                &[],
                1,
            ));
        }
        let config = CompilerConfig::default()
            .with_input_code(instruction_set! {
                Call(SYS_INPUT_LEN)
                BrIfEqz(3)
                Call(SYS_INPUT)
                Br(-3)
            })
            .with_output_code(instruction_set! {
                Call(SYS_OUTPUT_LEN)
                BrIfEqz(3)
                Call(SYS_OUTPUT)
                Br(-3)
            })
            .fuel_consume(false)
            .with_state(true)
            .with_global_start_index(GLOBAL_START_INDEX);
        let mut compiler =
            Compiler::new_with_linker(wasm_binary.as_slice(), config, Some(&import_linker))
                .unwrap();

        compiler.set_func_type_check_idx(self.func_type_check_idx.clone());

        compiler
            .translate(FuncOrExport::StateRouter(
                exports,
                instruction_set! {
                    Call(SYS_STATE)
                },
            ))
            .map_err(|err| TestError::Compiler(err))?;
        let rwasm_binary = compiler.finalize().unwrap();
        let reduced_module = ReducedModule::new(rwasm_binary.as_slice()).unwrap();
        let module_builder =
            reduced_module.to_module_builder(&self.engine, &import_linker, FuncType::new([], []));
        let module = module_builder.finish();

        self.store.data_mut().state = 1000;
        let instance = self
            .linker
            .instantiate(&mut self.store, &module)?
            .start(&mut self.store)?;
        if let Some(name) = name {
            self.instances.insert(name.to_string(), instance);
        }
        self.last_instance = Some(instance);

        if start_fn.is_some() {
            let router = self
                .main_router
                .iter()
                .find(|r| r.0 == "main")
                .expect("Main function idx not contain in state router");
            let func_name = router.0.clone();
            let func_index = router.1;
            self.store.data_mut().state = func_index.fn_index;
            self.invoke_with_state(name, func_name.as_str(), vec![])?;
        }

        Ok(())
    }

    pub fn compile_and_instantiate_method(
        &mut self,
        wasm_binary: &Vec<u8>,
        fn_name: &str,
    ) -> Result<Instance, TestError> {
        let mut config = Config::default();
        config.consume_fuel(false);
        println!("compiling function: {}", fn_name);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm_binary.as_slice())?;

        let mut import_linker = ImportLinker::default();

        import_linker.insert_function(ImportFunc::new_env(
            "spectest".to_string(),
            "print_i32".to_string(),
            SYS_PRINT_I32 as u16,
            &[ValueType::I32],
            &[],
            1,
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "spectest".to_string(),
            "print_i64".to_string(),
            SYS_PRINT_I64 as u16,
            &[ValueType::I64],
            &[],
            1,
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "spectest".to_string(),
            "print_f32".to_string(),
            SYS_PRINT_F32 as u16,
            &[ValueType::F32],
            &[],
            1,
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "spectest".to_string(),
            "print_f64".to_string(),
            SYS_PRINT_F64 as u16,
            &[ValueType::F64],
            &[],
            1,
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "spectest".to_string(),
            "print_i32_f32".to_string(),
            SYS_PRINT_I32_F32 as u16,
            &[ValueType::I32, ValueType::F32],
            &[],
            1,
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "spectest".to_string(),
            "print_f64_f64".to_string(),
            SYS_PRINT_F64_F64 as u16,
            &[ValueType::F64, ValueType::F64],
            &[],
            1,
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "spectest".to_string(),
            "print".to_string(),
            SYS_PRINT as u16,
            &[],
            &[],
            1,
        ));

        let mut compiler = Compiler::new_with_linker(
            wasm_binary.as_slice(),
            CompilerConfig::default().fuel_consume(false),
            Some(&import_linker),
        )
        .unwrap();
        compiler.set_func_type_check_idx(self.func_type_check_idx.clone());

        let elem = module
            .exports()
            .find(|export| export.name() == fn_name)
            .unwrap();

        if let Some(idx) = elem.index().into_func_idx() {
            compiler.translate(FuncOrExport::Func(idx)).unwrap();
        } else if let Some(ix) = module.get_global_init(elem.index()) {
            compiler.set_state(true);
            compiler.translate(FuncOrExport::Global(ix)).unwrap();
        }

        let rwasm_binary = compiler.finalize().unwrap();
        let reduced_module = ReducedModule::new(rwasm_binary.as_slice()).unwrap();

        let func_type = elem.ty().func();
        let global_type = elem
            .ty()
            .global()
            .map(|global| FuncType::new([], [global.content()]));
        let mut module_builder = reduced_module.to_module_builder(
            self.engine(),
            &import_linker,
            func_type.or(global_type.as_ref()).unwrap().clone(),
        );

        module_builder.remove_start();
        let module = module_builder.finish();
        let instance = self
            .linker
            .instantiate(&mut self.store, &module)?
            .start(&mut self.store)?;
        self.last_instance = Some(instance);
        Ok(instance)
    }

    /// Loads the Wasm module instance with the given name.
    ///
    /// # Errors
    ///
    /// If there is no registered module instance with the given name.
    pub fn instance_by_name(&self, name: &str) -> Result<Instance, TestError> {
        self.instances
            .get(name)
            .copied()
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
            .unwrap_or_else(|| self.last_instance.ok_or(TestError::NoModuleInstancesFound))
    }

    /// Registers the given [`Instance`] with the given `name` and sets it as the last instance.
    pub fn register_instance(&mut self, name: &str, instance: Instance) {
        if self.instances.get(name).is_some() {
            // Already registered the instance.
            return;
        }
        self.instances.insert(name.to_string(), instance);
        self.last_instance = Some(instance);
        for export in instance.exports(&self.store) {
            self.linker
                .define(name, export.name(), export.clone().into_extern())
                .unwrap_or_else(|error| {
                    let field_name = export.name();
                    let export = export.clone().into_extern();
                    panic!(
                        "failed to define export {name}::{field_name}: \
                        {export:?}: {error}",
                    )
                });
        }
        self.last_instance = Some(instance);
    }

    pub fn compile_exports_module(&mut self) -> Result<Vec<(String, Instance)>, TestError> {
        if self.last_exports.is_none() {
            return Ok(vec![]);
        }

        self.last_exports
            .as_ref()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .map(|last_export| {
                let wasm_binary = self.binaries.get(&last_export).unwrap().clone();
                self.compile_and_instantiate_method(&wasm_binary, &last_export)
                    .map(|instance| (last_export, instance))
            })
            .collect()
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
        _module_name: Option<&str>,
        func_name: &str,
        args: &[Value],
    ) -> Result<&[Value], TestError> {
        if func_name == "32_good5" && args.len() > 0 && args[0].i32().unwrap() == 65508 {
            println!("{}", func_name)
        }
        let wasm_binary = self.binaries.get(&func_name.to_string()).unwrap().clone();
        let instance = self.compile_and_instantiate_method(&wasm_binary, func_name)?;
        let func = instance
            .get_export(&self.store, "main")
            .and_then(Extern::into_func)
            .unwrap();
        println!("testing {} with args {:?}", func_name, args);
        let len_results = func.ty(&self.store).results().len();
        self.results.clear();
        self.results.resize(len_results, Value::I32(0));
        func.call(&mut self.store, args, &mut self.results)?;
        Ok(&self.results)
    }

    pub fn invoke_with_state(
        &mut self,
        module_name: Option<&str>,
        func_name: &str,
        args: Vec<Value>,
    ) -> Result<Vec<Value>, TestError> {
        let instance = if let Some(module_name) = module_name {
            self.instances.get(module_name)
        } else {
            self.last_instance.as_ref()
        }
        .unwrap();

        let func = instance
            .get_export(&self.store, "main")
            .and_then(Extern::into_func)
            .unwrap();
        let func_ty = &self.main_router.get(func_name).unwrap().fn_type;

        println!(
            "testing {} func ty: {:?}, with args {:?}, len_res: {:?}",
            func_name,
            func_ty,
            args,
            func.ty(&self.store).results()
        );
        self.results.clear();
        self.store.data_mut().input = args.into_iter().rev().map(|v| v.into()).collect();
        self.store
            .data_mut()
            .clear_ouput(func_ty.results().len() as u32);
        func.call(&mut self.store, &[], &mut self.results)?;
        Ok(self
            .store
            .data_mut()
            .output()
            .iter()
            .rev()
            .zip(func_ty.results())
            .map(|(res, tp)| res.with_type(*tp))
            .collect())
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
        let instance = self.instance_by_name_or_last(module_name)?;
        let global = instance
            .get_export(&self.store, global_name)
            .and_then(Extern::into_global)
            .ok_or_else(|| TestError::GlobalNotFound {
                module_name: module_name.map(|name| name.to_string()),
                global_name: global_name.to_string(),
            })?;
        let value = global.get(&self.store);
        Ok(value)
    }
}
