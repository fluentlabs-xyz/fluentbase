use crate::{
    macros::{forward_call, forward_call_args},
    Error,
    SysFuncIdx,
};
use fluentbase_rwasm::{
    common::{Trap, ValueType},
    engine::Tracer,
    rwasm::{ImportFunc, ImportLinker, ReducedModule},
    AsContextMut,
    Caller,
    Config,
    Engine,
    FuelConsumptionMode,
    Func,
    Linker,
    Module,
    Store,
};

#[derive(Default, Debug, Clone)]
pub struct RuntimeContext {
    pub(crate) state: u32,
    pub(crate) exit_code: i32,
    pub(crate) input: Vec<u8>,
    pub(crate) output: Vec<u8>,
    pub(crate) timestamp: u64,
}

impl RuntimeContext {
    pub fn new(input_data: &[u8], state: u32) -> Self {
        Self {
            input: input_data.to_vec(),
            state,
            ..Default::default()
        }
    }

    pub(crate) fn return_data(&mut self, value: &[u8]) {
        self.output.extend(value);
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn input(&self) -> &Vec<u8> {
        &self.input
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.output
    }
}

#[derive(Debug)]
pub struct ExecutionResult {
    store: Store<RuntimeContext>,
    bytecode: Vec<u8>,
}

impl ExecutionResult {
    pub fn new(store: Store<RuntimeContext>, bytecode: Vec<u8>) -> Self {
        Self { store, bytecode }
    }

    pub fn bytecode(&self) -> &Vec<u8> {
        &self.bytecode
    }

    pub fn tracer(&self) -> &Tracer {
        self.store.tracer()
    }

    pub fn data(&self) -> &RuntimeContext {
        self.store.data()
    }

    pub fn data_mut(&mut self) -> &mut RuntimeContext {
        self.store.data_mut()
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
        // Fluentbase sys calls
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_halt".to_string(),
            SysFuncIdx::SYS_HALT as u16,
            &[ValueType::I32; 1],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_state".to_string(),
            SysFuncIdx::SYS_STATE as u16,
            &[],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_write".to_string(),
            SysFuncIdx::SYS_WRITE as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_read".to_string(),
            SysFuncIdx::SYS_READ as u16,
            &[ValueType::I32; 3],
            &[],
        ));
        // WASI sys calls
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "proc_exit".to_string(),
            SysFuncIdx::WASI_PROC_EXIT as u16,
            &[ValueType::I32; 1],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "fd_write".to_string(),
            SysFuncIdx::WASI_FD_WRITE as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "environ_sizes_get".to_string(),
            SysFuncIdx::WASI_ENVIRON_SIZES_GET as u16,
            &[ValueType::I32; 2],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "environ_get".to_string(),
            SysFuncIdx::WASI_ENVIRON_GET as u16,
            &[ValueType::I32; 2],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "args_sizes_get".to_string(),
            SysFuncIdx::WASI_ARGS_SIZES_GET as u16,
            &[ValueType::I32; 0],
            &[ValueType::I32; 2],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "args_get".to_string(),
            SysFuncIdx::WASI_ARGS_GET as u16,
            &[ValueType::I32; 2],
            &[ValueType::I32; 1],
        ));
        // RWASM sys calls
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_rwasm_transact".to_string(),
            SysFuncIdx::RWASM_TRANSACT as u16,
            &[ValueType::I32; 6],
            &[ValueType::I32; 1],
        ));
        // EVM sys calls
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_evm_stop".to_string(),
            SysFuncIdx::EVM_STOP as u16,
            &[ValueType::I32; 0],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_evm_return".to_string(),
            SysFuncIdx::EVM_RETURN as u16,
            &[ValueType::I32; 2],
            &[],
        ));

        // zktrie
        // zktrie_open
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_open".to_string(),
            SysFuncIdx::ZKTRIE_OPEN as u16,
            &[ValueType::I32; 5],
            &[],
        ));
        // account updates
        // zktrie_update_nonce
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_nonce".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_NONCE as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // zktrie_update_balance
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_balance".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_BALANCE as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // zktrie_update_storage_root
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_storage_root".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_STORAGE_ROOT as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // zktrie_update_code_hash
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_code_hash".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_CODE_HASH as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // zktrie_update_code_size
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_code_size".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_CODE_SIZE as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // account gets
        // zktrie_get_nonce
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_nonce".to_string(),
            SysFuncIdx::ZKTRIE_GET_NONCE as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // zktrie_get_balance
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_balance".to_string(),
            SysFuncIdx::ZKTRIE_GET_BALANCE as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // zktrie_get_storage_root
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_storage_root".to_string(),
            SysFuncIdx::ZKTRIE_GET_STORAGE_ROOT as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // zktrie_get_code_hash
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_code_hash".to_string(),
            SysFuncIdx::ZKTRIE_GET_CODE_HASH as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // zktrie_get_code_size
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_code_size".to_string(),
            SysFuncIdx::ZKTRIE_GET_CODE_SIZE as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // store updates
        // zktrie_update_store
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_store".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_STORE as u16,
            &[ValueType::I32; 2],
            &[],
        ));
        // store gets
        // zktrie_get_store
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_store".to_string(),
            SysFuncIdx::ZKTRIE_GET_STORE as u16,
            &[ValueType::I32; 2],
            &[],
        ));

        import_linker
    }

    pub fn run(rwasm_binary: &[u8], input_data: &[u8]) -> Result<ExecutionResult, Error> {
        let import_linker = Self::new_linker();
        Self::run_with_input(rwasm_binary, input_data, &import_linker, true)
    }

    pub fn run_with_input(
        rwasm_binary: &[u8],
        input_data: &[u8],
        import_linker: &ImportLinker,
        catch_trap: bool,
    ) -> Result<ExecutionResult, Error> {
        Self::run_with_context(
            rwasm_binary,
            RuntimeContext::new(input_data, 0),
            import_linker,
            catch_trap,
        )
    }

    pub fn run_with_context(
        rwasm_binary: &[u8],
        runtime_context: RuntimeContext,
        import_linker: &ImportLinker,
        catch_trap: bool,
    ) -> Result<ExecutionResult, Error> {
        let mut config = Config::default();
        let fuel_enabled = true;
        if fuel_enabled {
            config.fuel_consumption_mode(FuelConsumptionMode::Eager);
            config.consume_fuel(true);
        }
        let engine = Engine::new(&config);

        let reduced_module = ReducedModule::new(rwasm_binary).map_err(Into::<Error>::into)?;
        let module = reduced_module.to_module(&engine, import_linker);
        let linker = Linker::<RuntimeContext>::new(&engine);
        let mut store = Store::<RuntimeContext>::new(&engine, runtime_context);

        if fuel_enabled {
            store.add_fuel(100_000).unwrap();
        }

        #[allow(unused_mut)]
        let mut res = Self {
            engine,
            module,
            linker,
            store,
        };

        forward_call!(res, "env", "_sys_halt", fn sys_halt(exit_code: u32) -> ());
        forward_call!(res, "env", "_sys_state", fn sys_state() -> u32);
        forward_call!(res, "env", "_sys_read", fn sys_read(target: u32, offset: u32, length: u32) -> ());
        forward_call!(res, "env", "_sys_write", fn sys_write(offset: u32, length: u32) -> ());

        forward_call!(res, "wasi_snapshot_preview1", "proc_exit", fn wasi_proc_exit(exit_code: i32) -> ());
        forward_call!(res, "wasi_snapshot_preview1", "fd_write", fn wasi_fd_write(fd: i32, iovs_ptr: i32, iovs_len: i32, rp0_ptr: i32) -> i32);
        forward_call!(res, "wasi_snapshot_preview1", "environ_sizes_get", fn wasi_environ_sizes_get(rp0_ptr: i32, rp1_ptr: i32) -> i32);
        forward_call!(res, "wasi_snapshot_preview1", "environ_get", fn wasi_environ_get(environ: i32, environ_buffer: i32) -> i32);
        forward_call!(res, "wasi_snapshot_preview1", "args_sizes_get", fn wasi_args_sizes_get(argv_len: i32, argv_buffer_len: i32) -> i32);
        forward_call!(res, "wasi_snapshot_preview1", "args_get", fn wasi_args_get(argv: i32, argv_buffer: i32) -> i32);

        forward_call!(res, "env", "_rwasm_transact", fn rwasm_transact(code_offset: i32, code_len: i32, input_offset: i32, input_len: i32, output_offset: i32, output_len: i32) -> i32);

        forward_call!(res, "env", "_evm_stop", fn evm_stop() -> ());
        forward_call!(res, "env", "_evm_return", fn evm_return(offset: u32, length: u32) -> ());

        forward_call!(res, "env", "zktrie_open", fn zktrie_open(root_offset: i32, root_len: i32, keys_offset: i32, leafs_offset: i32, accounts_count: i32) -> ());
        forward_call!(res, "env", "zktrie_update_nonce", fn zktrie_update_nonce(offset: i32, length: i32) -> ());
        forward_call!(res, "env", "zktrie_get_nonce", fn zktrie_get_nonce(key_offset: i32, output_offset: i32) -> ());
        forward_call!(res, "env", "zktrie_update_balance", fn zktrie_update_balance(offset: i32, length: i32) -> ());
        forward_call!(res, "env", "zktrie_get_balance", fn zktrie_get_balance(key_offset: i32, output_offset: i32) -> ());
        forward_call!(res, "env", "zktrie_update_storage_root", fn zktrie_update_storage_root(offset: i32, length: i32) -> ());
        forward_call!(res, "env", "zktrie_get_storage_root", fn zktrie_get_storage_root(key_offset: i32, output_offset: i32) -> ());
        forward_call!(res, "env", "zktrie_update_code_hash", fn zktrie_update_code_hash(offset: i32, length: i32) -> ());
        forward_call!(res, "env", "zktrie_get_code_hash", fn zktrie_get_code_hash(key_offset: i32, output_offset: i32) -> ());
        forward_call!(res, "env", "zktrie_update_code_size", fn zktrie_update_code_size(offset: i32, length: i32) -> ());
        forward_call!(res, "env", "zktrie_get_code_size", fn zktrie_get_code_size(key_offset: i32, output_offset: i32) -> ());
        forward_call!(res, "env", "zktrie_update_store", fn zktrie_update_store(key_offset: i32, value_offset: i32) -> ());
        forward_call!(res, "env", "zktrie_get_store", fn zktrie_get_store(key_offset: i32, output_offset: i32) -> ());

        let result = res
            .linker
            .instantiate(&mut res.store, &res.module)
            .map_err(Into::<Error>::into)?
            .start(&mut res.store);

        // we need to fix logs, because we lost information about instr meta during conversion
        let tracer = res.store.tracer_mut();
        let call_id = tracer.logs.first().map(|v| v.call_id).unwrap_or_default();
        for log in tracer.logs.iter_mut() {
            if log.call_id != call_id {
                continue;
            }
            let instr = reduced_module.bytecode().get(log.index).unwrap();
            log.opcode = *instr;
        }

        let mut execution_result = ExecutionResult::new(res.store, rwasm_binary.to_vec());

        if !catch_trap {
            result?;
            return Ok(execution_result);
        }

        if let Err(ref err) = result {
            let exit_code = match err {
                fluentbase_rwasm::Error::Trap(trap) => {
                    if trap.i32_exit_status().is_none() {
                        result?;
                        return Ok(execution_result);
                    }
                    trap.i32_exit_status().unwrap()
                }
                _ => {
                    result?;
                    return Ok(execution_result);
                }
            };
            execution_result.data_mut().exit_code = exit_code;
        }

        Ok(execution_result)
    }
}
