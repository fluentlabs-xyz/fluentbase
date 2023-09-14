use super::{TestDescriptor, TestError, TestProfile, TestSpan};
use anyhow::Result;
use fluentbase_rwasm::{
    common::{ValueType, F32, F64},
    rwasm::{Compiler, DefaultImportHandler, ImportLinker, ReducedModule},
    Config,
    Engine,
    Extern,
    Func,
    Global,
    Instance,
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
use std::collections::HashMap;
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
}

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
        let global_i32 = Global::new(&mut store, Value::I32(666), Mutability::Const);
        let global_i64 = Global::new(&mut store, Value::I64(666), Mutability::Const);
        let global_f32 = Global::new(&mut store, Value::F32(666.0.into()), Mutability::Const);
        let global_f64 = Global::new(&mut store, Value::F64(666.0.into()), Mutability::Const);
        let print = Func::wrap(&mut store, || {
            println!("print");
        });
        let print_i32 = Func::wrap(&mut store, |value: i32| {
            println!("print: {value}");
        });
        let print_i64 = Func::wrap(&mut store, |value: i64| {
            println!("print: {value}");
        });
        let print_f32 = Func::wrap(&mut store, |value: F32| {
            println!("print: {value:?}");
        });
        let print_f64 = Func::wrap(&mut store, |value: F64| {
            println!("print: {value:?}");
        });
        let print_i32_f32 = Func::wrap(&mut store, |v0: i32, v1: F32| {
            println!("print: {v0:?} {v1:?}");
        });
        let print_f64_f64 = Func::wrap(&mut store, |v0: F64, v1: F64| {
            println!("print: {v0:?} {v1:?}");
        });
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
        TestContext {
            engine,
            linker,
            store,
            modules: Vec::new(),
            instances: HashMap::new(),
            last_instance: None,
            profile: TestProfile::default(),
            results: Vec::new(),
            descriptor,
            binaries: HashMap::new(),
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
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm_binary.as_slice())?;
        let elem = module
            .exports()
            .find(|export| export.name() == fn_name)
            .unwrap();
        let import_linker = ImportLinker::default();
        let mut compiler = Compiler::new(wasm_binary.as_slice()).unwrap();
        compiler
            .translate(Some(elem.index().into_func_idx().unwrap()))
            .unwrap();
        let rwasm_binary = compiler.finalize().unwrap();
        let reduced_module = ReducedModule::new(rwasm_binary.as_slice()).unwrap();
        let func_type = elem.ty().func().unwrap();
        let mut module_builder =
            reduced_module.to_module_builder(self.engine(), &import_linker, func_type.clone());
        module_builder.remove_start();
        let module = module_builder.finish();
        let instance = self
            .linker
            .instantiate(&mut self.store, &module)?
            .start(&mut self.store)?;
        Ok(instance)
    }

    pub fn compile_and_instantiate_(
        &mut self,
        mut module: wast::core::Module,
    ) -> Result<(), TestError> {
        let module_name = module.id.map(|id| id.name());
        let wasm = module.encode().unwrap_or_else(|error| {
            panic!(
                "encountered unexpected failure to encode `.wast` module into `.wasm`:{}: {}",
                self.test_path(),
                error
            )
        });
        let module = Module::new(self.engine(), &wasm[..])?;
        let instance_pre = self.linker.instantiate(&mut self.store, &module)?;
        let instance = instance_pre.start(&mut self.store)?;
        self.modules.push(module);
        if let Some(module_name) = module_name {
            self.instances.insert(module_name.to_string(), instance);
            for export in instance.exports(&self.store) {
                self.linker
                    .define(module_name, export.name(), export.into_extern())?;
            }
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
        if func_name == "as-block-first" {
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
