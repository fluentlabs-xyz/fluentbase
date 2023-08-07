use crate::{
    platform::{IMPORT_SYS_HALT, IMPORT_SYS_READ, IMPORT_SYS_WRITE},
    Error,
    StateHandler,
};
use fluentbase_rwasm::{
    common::{Trap, ValueType},
    rwasm::{ImportFunc, ImportLinker, ReducedModule},
    AsContextMut,
    Caller,
    Config,
    Engine,
    Extern,
    Func,
    Linker,
    Module,
    Store,
};

pub struct RuntimeContext {
    input: Vec<u8>,
}

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
            "_sys_read".to_string(),
            IMPORT_SYS_READ,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_write".to_string(),
            IMPORT_SYS_WRITE,
            &[ValueType::I32; 3],
            &[],
        ));

        import_linker
    }

    pub fn run(rwasm_binary: &[u8], input_data: &[u8]) -> Result<Self, Error> {
        let import_linker = Self::new_linker();
        Self::run_with_linker(rwasm_binary, input_data, &import_linker)
    }

    pub fn run_with_linker(
        rwasm_binary: &[u8],
        input_data: &[u8],
        import_linker: &ImportLinker,
    ) -> Result<Self, Error> {
        let config = Config::default();
        let engine = Engine::new(&config);

        let runtime_context = RuntimeContext {
            input: input_data.to_vec(),
        };

        let reduced_module = ReducedModule::new(rwasm_binary).map_err(Into::<Error>::into)?;
        let module = reduced_module
            .to_module(&engine, import_linker)
            .map_err(Into::<Error>::into)?;
        let mut linker = Linker::<RuntimeContext>::new(&engine);
        let mut store = Store::<RuntimeContext>::new(&engine, runtime_context);

        #[allow(unused_mut)]
        let mut res = Self {
            engine,
            module,
            linker,
            store,
        };

        res.linker.define(
            "env",
            "_sys_halt",
            Func::wrap(
                res.store.as_context_mut(),
                |caller: Caller<'_, RuntimeContext>, exit_code: u32| -> Result<(), Trap> {
                    Err(Trap::i32_exit(exit_code as i32))
                },
            ),
        )?;
        res.linker.define(
            "env",
            "_sys_write",
            Func::wrap(
                res.store.as_context_mut(),
                |mut caller: Caller<'_, RuntimeContext>, source: u32, offset: u32, length: u32| -> Result<(), Trap> {
                    let memory = caller.get_export("memory").unwrap();
                    let memory = match memory {
                        Extern::Memory(memory) => memory,
                        _ => unreachable!("there is no memory export inside"),
                    };
                    let input = &caller.data().input;
                    // let mut memory = memory.data_mut(caller.as_context());
                    // TODO: "add overflow checks"
                    // memory[(source as usize)..((source + length) as usize)].clone_from_slice(input.as_slice());
                    Ok(())
                },
            ),
        )?;
        res.linker.define(
            "env",
            "_sys_read",
            Func::wrap(
                res.store.as_context_mut(),
                |mut caller: Caller<'_, RuntimeContext>, target: u32, offset: u32, length: u32| -> Result<u32, Trap> {
                    let memory = caller.get_export("memory").unwrap();
                    let memory = match memory {
                        Extern::Memory(memory) => memory,
                        _ => unreachable!("there is no memory export inside"),
                    };
                    let input = caller.data().input.clone();
                    let memory = memory.data_mut(caller.as_context_mut());
                    let length = if length > input.len() as u32 {
                        input.len() as u32
                    } else {
                        length
                    };
                    memory[(target as usize)..((target + length) as usize)].clone_from_slice(input.as_slice());
                    Ok(length)
                },
            ),
        )?;

        // link_call!("_sys_halt", fn sys_halt(exit_code: u32));
        // link_call!("_sys_write", fn sys_write(offset: u32, length: u32));
        // link_call!("_sys_read", fn sys_read(target: u32, offset: u32, length: u32));

        res.linker
            .instantiate(&mut res.store, &res.module)
            .map_err(Into::<Error>::into)?
            .start(&mut res.store)?;

        Ok(res)
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
