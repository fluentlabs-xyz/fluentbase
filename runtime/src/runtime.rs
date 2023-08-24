use crate::{
    macros::{forward_call, forward_call_args},
    platform::{
        IMPORT_EVM_RETURN,
        IMPORT_EVM_STOP,
        IMPORT_SYS_HALT,
        IMPORT_SYS_READ,
        IMPORT_SYS_WRITE,
    },
    Error,
};
use fluentbase_rwasm::{
    common::{Trap, ValueType},
    rwasm::{ImportFunc, ImportLinker, ReducedModule},
    AsContextMut,
    Caller,
    Config,
    Engine,
    Func,
    Linker,
    Module,
    Store,
};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RuntimeContext {
    pub(crate) input: Vec<u8>,
    pub(crate) output: Vec<u8>,
}

impl RuntimeContext {
    pub(crate) fn return_data(&mut self, value: &[u8]) {
        self.output.resize(value.len(), 0);
        self.output.copy_from_slice(value);
    }
}

#[allow(dead_code)]
pub struct Runtime {
    engine: Engine,
    module: Module,
    linker: Linker<RuntimeContext>,
    store: Store<RuntimeContext>,
}

impl Runtime {
    pub fn new_linker() -> ImportLinker {
        let mut import_linker = ImportLinker::default();

        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_halt".to_string(),
            IMPORT_SYS_HALT,
            &[ValueType::I32; 1],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_write".to_string(),
            IMPORT_SYS_WRITE,
            &[ValueType::I32; 3],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_read".to_string(),
            IMPORT_SYS_READ,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_evm_stop".to_string(),
            IMPORT_EVM_STOP,
            &[ValueType::I32; 0],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_evm_return".to_string(),
            IMPORT_EVM_RETURN,
            &[ValueType::I32; 2],
            &[],
        ));

        import_linker
    }

    pub fn run(rwasm_binary: &[u8], input_data: &[u8]) -> Result<RuntimeContext, Error> {
        let import_linker = Self::new_linker();
        Self::run_with_linker(rwasm_binary, input_data, &import_linker)
    }

    pub fn run_with_linker(
        rwasm_binary: &[u8],
        input_data: &[u8],
        import_linker: &ImportLinker,
    ) -> Result<RuntimeContext, Error> {
        let config = Config::default();
        let engine = Engine::new(&config);

        let runtime_context = RuntimeContext {
            input: input_data.to_vec(),
            output: Vec::new(),
        };

        let reduced_module = ReducedModule::new(rwasm_binary).map_err(Into::<Error>::into)?;
        let module = reduced_module.to_module(&engine, import_linker);
        let linker = Linker::<RuntimeContext>::new(&engine);
        let store = Store::<RuntimeContext>::new(&engine, runtime_context);

        #[allow(unused_mut)]
        let mut res = Self {
            engine,
            module,
            linker,
            store,
        };

        forward_call!(res, "env", "_sys_halt", fn sys_halt(exit_code: u32) -> ());
        forward_call!(res, "env", "_sys_read", fn sys_read(target: u32, offset: u32, length: u32) -> u32);

        forward_call!(res, "env", "_evm_stop", fn evm_stop() -> ());
        forward_call!(res, "env", "_evm_return", fn evm_return(offset: u32, length: u32) -> ());

        res.linker
            .instantiate(&mut res.store, &res.module)
            .map_err(Into::<Error>::into)?
            .start(&mut res.store)?;

        Ok(res.store.data().clone())
    }
}

// impl StateHandler<RuntimeState<'_>> for Runtime<'_> {
//     // sys calls
//     fn sys_halt(&mut self, caller: &Caller<RuntimeState>, _exit_code: u32) {}
//     fn sys_write(&mut self, caller: &Caller<RuntimeState>, _offset: u32, _length: u32) {}
//     fn sys_read(&mut self, caller: &Caller<RuntimeState>, target: u32, offset: u32, length: u32)
// {}     // evm calls
//     fn evm_return(&mut self, caller: &Caller<RuntimeState>, _offset: u32, _length: u32) {}
// }
