use crate::{
    macros::{forward_call, forward_call_args},
    ExitCode,
    RuntimeError,
    SysFuncIdx,
    RECURSIVE_MAX_DEPTH,
    STACK_MAX_HEIGHT,
};
use fluentbase_rwasm::{
    common::{Trap, ValueType},
    engine::Tracer,
    rwasm::{ImportFunc, ImportLinker, InstructionSet, ReducedModule, ReducedModuleError},
    AsContextMut,
    Caller,
    Config,
    Engine,
    FuelConsumptionMode,
    Func,
    FuncType,
    Instance,
    Linker,
    Module,
    StackLimits,
    Store,
};
use std::mem::take;

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    // context inputs
    pub(crate) bytecode: Vec<u8>,
    pub(crate) fuel_limit: u32,
    pub(crate) state: u32,
    pub(crate) catch_trap: bool,
    pub(crate) input: Vec<Vec<u8>>,
    // context outputs
    pub(crate) exit_code: i32,
    pub(crate) output: Vec<u8>,
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self {
            bytecode: vec![],
            fuel_limit: 0,
            state: 0,
            catch_trap: true,
            input: vec![],
            exit_code: 0,
            output: vec![],
        }
    }
}

impl RuntimeContext {
    pub fn new<I: Into<Vec<u8>>>(bytecode: I) -> Self {
        Self {
            bytecode: bytecode.into(),
            ..Default::default()
        }
    }

    pub fn with_input(mut self, input_data: &[u8]) -> Self {
        self.input = vec![input_data.to_vec()];
        self
    }

    pub fn with_state(mut self, state: u32) -> Self {
        self.state = state;
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

    pub(crate) fn extend_return_data(&mut self, value: &[u8]) {
        self.output.extend(value);
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn input(&self, argc: usize) -> &Vec<u8> {
        // TODO: "add overflow check here"
        self.input.get(argc).as_ref().unwrap()
    }

    pub fn input_count(&self) -> u32 {
        self.input.len() as u32
    }

    pub fn input_size(&self) -> u32 {
        self.input.iter().map(|v| v.len() as u32).sum::<u32>()
    }

    pub fn argv_buffer(&self) -> Vec<u8> {
        self.input.iter().fold(Vec::new(), |mut a, b| {
            a.extend(b);
            a
        })
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.output
    }
}

#[derive(Debug)]
pub struct ExecutionResult {
    runtime_context: RuntimeContext,
    tracer: Tracer,
}

impl ExecutionResult {
    pub fn cloned(store: &Store<RuntimeContext>) -> Self {
        Self {
            runtime_context: store.data().clone(),
            tracer: store.tracer().clone(),
        }
    }

    pub fn taken(store: &mut Store<RuntimeContext>) -> Self {
        Self {
            runtime_context: take(store.data_mut()),
            tracer: take(store.tracer_mut()),
        }
    }

    pub fn bytecode(&self) -> &Vec<u8> {
        &self.runtime_context.bytecode
    }

    pub fn tracer(&self) -> &Tracer {
        &self.tracer
    }

    pub fn data(&self) -> &RuntimeContext {
        &self.runtime_context
    }
}

#[allow(dead_code)]
pub struct Runtime {
    engine: Engine,
    bytecode: InstructionSet,
    module: Module,
    linker: Linker<RuntimeContext>,
    store: Store<RuntimeContext>,
    instance: Instance,
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
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_input".to_string(),
            SysFuncIdx::SYS_INPUT as u16,
            &[ValueType::I32; 4],
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
            &[ValueType::I32; 2],
            &[ValueType::I32; 1],
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
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_open".to_string(),
            SysFuncIdx::ZKTRIE_OPEN as u16,
            &[ValueType::I32; 0],
            &[ValueType::I32; 0],
        ));
        // account updates
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_nonce".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_NONCE as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_balance".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_BALANCE as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_storage_root".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_STORAGE_ROOT as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_code_hash".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_CODE_HASH as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_code_size".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_CODE_SIZE as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        // account gets
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_nonce".to_string(),
            SysFuncIdx::ZKTRIE_GET_NONCE as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_balance".to_string(),
            SysFuncIdx::ZKTRIE_GET_BALANCE as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_storage_root".to_string(),
            SysFuncIdx::ZKTRIE_GET_STORAGE_ROOT as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_code_hash".to_string(),
            SysFuncIdx::ZKTRIE_GET_CODE_HASH as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_code_size".to_string(),
            SysFuncIdx::ZKTRIE_GET_CODE_SIZE as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        // store updates
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_update_store".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_STORE as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        // store gets
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "zktrie_get_store".to_string(),
            SysFuncIdx::ZKTRIE_GET_STORE as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "mpt_open".to_string(),
            SysFuncIdx::MPT_OPEN as u16,
            &[ValueType::I32; 0],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "mpt_update".to_string(),
            SysFuncIdx::MPT_UPDATE as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "mpt_get".to_string(),
            SysFuncIdx::MPT_GET as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "mpt_get_root".to_string(),
            SysFuncIdx::MPT_GET_ROOT as u16,
            &[ValueType::I32; 1],
            &[ValueType::I32; 1],
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "crypto_keccak".to_string(),
            SysFuncIdx::CRYPTO_KECCAK as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "crypto_poseidon".to_string(),
            SysFuncIdx::CRYPTO_POSEIDON as u16,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "crypto_poseidon_with_domain".to_string(),
            SysFuncIdx::CRYPTO_POSEIDON_WITH_DOMAIN as u16,
            &[ValueType::I32; 4],
            &[ValueType::I32; 1],
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "crypto_secp256k1_verify".to_string(),
            SysFuncIdx::CRYPTO_SECP256K1_VERIFY as u16,
            &[ValueType::I32; 7],
            &[ValueType::I32; 1],
        ));

        import_linker
    }

    pub fn run(rwasm_binary: &[u8], input_data: &[u8]) -> Result<ExecutionResult, RuntimeError> {
        let runtime_context = RuntimeContext::new(rwasm_binary)
            .with_input(input_data)
            .with_catch_trap(true);
        let import_linker = Self::new_linker();
        Self::run_with_context(runtime_context, &import_linker)
    }

    pub fn run_with_context(
        mut runtime_context: RuntimeContext,
        import_linker: &ImportLinker,
    ) -> Result<ExecutionResult, RuntimeError> {
        let catch_error = runtime_context.catch_trap;
        let mut runtime = Self::new(runtime_context.clone(), import_linker);
        if catch_error && runtime.is_err() {
            runtime_context.exit_code = Self::catch_trap(runtime.err().unwrap());
            return Ok(ExecutionResult {
                runtime_context,
                tracer: Default::default(),
            });
        }
        runtime?.call()
    }

    pub fn new(
        runtime_context: RuntimeContext,
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
            let module_builder =
                reduced_module.to_module_builder(&engine, import_linker, FuncType::new([], []));
            (module_builder.finish(), reduced_module.bytecode().clone())
        };

        let mut linker = Linker::<RuntimeContext>::new(&engine);
        let mut store = Store::<RuntimeContext>::new(&engine, runtime_context);

        if fuel_limit > 0 {
            store.add_fuel(fuel_limit as u64).unwrap();
        }

        Self::register_bindings(&mut linker, &mut store);

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(Into::<RuntimeError>::into)?
            .start(&mut store)
            .map_err(Into::<RuntimeError>::into)?;

        let result = Self {
            engine,
            bytecode,
            module,
            linker,
            store,
            instance,
        };

        Ok(result)
    }

    pub fn call(&mut self) -> Result<ExecutionResult, RuntimeError> {
        let func =
            self.instance
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

    fn register_bindings(linker: &mut Linker<RuntimeContext>, store: &mut Store<RuntimeContext>) {
        // sys
        forward_call!(linker, store, "env", "_sys_halt", fn sys_halt(exit_code: u32) -> ());
        forward_call!(linker, store, "env", "_sys_state", fn sys_state() -> u32);
        forward_call!(linker, store, "env", "_sys_read", fn sys_read(target: u32, offset: u32, length: u32) -> ());
        forward_call!(linker, store, "env", "_sys_input", fn sys_input(index: u32, target: u32, offset: u32, length: u32) -> ());
        forward_call!(linker, store, "env", "_sys_write", fn sys_write(offset: u32, length: u32) -> ());
        // wasi
        forward_call!(linker, store, "wasi_snapshot_preview1", "proc_exit", fn wasi_proc_exit(exit_code: i32) -> ());
        forward_call!(linker, store, "wasi_snapshot_preview1", "fd_write", fn wasi_fd_write(fd: i32, iovs_ptr: i32, iovs_len: i32, rp0_ptr: i32) -> i32);
        forward_call!(linker, store, "wasi_snapshot_preview1", "environ_sizes_get", fn wasi_environ_sizes_get(rp0_ptr: i32, rp1_ptr: i32) -> i32);
        forward_call!(linker, store, "wasi_snapshot_preview1", "environ_get", fn wasi_environ_get(environ: i32, environ_buffer: i32) -> i32);
        forward_call!(linker, store, "wasi_snapshot_preview1", "args_sizes_get", fn wasi_args_sizes_get(argc_ptr: i32, argv_ptr: i32) -> i32);
        forward_call!(linker, store, "wasi_snapshot_preview1", "args_get", fn wasi_args_get(argv_ptrs_ptr: i32, argv_buff_ptr: i32) -> i32);
        // rwasm
        forward_call!(linker, store, "env", "_rwasm_transact", fn rwasm_transact(code_offset: i32, code_len: i32, input_offset: i32, input_len: i32, output_offset: i32, output_len: i32) -> i32);
        // evm (orphaned)
        forward_call!(linker, store, "env", "_evm_stop", fn evm_stop() -> ());
        forward_call!(linker, store, "env", "_evm_return", fn evm_return(offset: u32, length: u32) -> ());
        forward_call!(linker, store, "env", "_evm_block_number", fn evm_block_number(ptr: u32) -> ());

        // zktrie
        forward_call!(linker, store, "env", "zktrie_open", fn zktrie_open() -> ());
        forward_call!(linker, store, "env", "zktrie_update_nonce", fn zktrie_update_nonce(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) -> ());
        forward_call!(linker, store, "env", "zktrie_get_nonce", fn zktrie_get_nonce(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "zktrie_update_balance", fn zktrie_update_balance(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) -> ());
        forward_call!(linker, store, "env", "zktrie_get_balance", fn zktrie_get_balance(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "zktrie_update_storage_root", fn zktrie_update_storage_root(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) -> ());
        forward_call!(linker, store, "env", "zktrie_get_storage_root", fn zktrie_get_storage_root(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "zktrie_update_code_hash", fn zktrie_update_code_hash(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) -> ());
        forward_call!(linker, store, "env", "zktrie_get_code_hash", fn zktrie_get_code_hash(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "zktrie_update_code_size", fn zktrie_update_code_size(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) -> ());
        forward_call!(linker, store, "env", "zktrie_get_code_size", fn zktrie_get_code_size(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "zktrie_update_store", fn zktrie_update_store(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) -> ());
        forward_call!(linker, store, "env", "zktrie_get_store", fn zktrie_get_store(key_offset: i32, key_len: i32, output_offset: i32) -> i32);

        // mpt
        forward_call!(linker, store, "env", "mpt_open", fn mpt_open() -> ());
        forward_call!(linker, store, "env", "mpt_update", fn mpt_update(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) -> ());
        forward_call!(linker, store, "env", "mpt_get", fn mpt_get(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "mpt_get_root", fn mpt_get_root(output_offset: i32) -> i32);

        // crypto
        forward_call!(linker, store, "env", "crypto_keccak", fn crypto_keccak(data_offset: i32, data_len: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "crypto_poseidon", fn crypto_poseidon(data_offset: i32, data_len: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "crypto_poseidon_with_domain", fn crypto_poseidon_with_domain(fa_offset: i32, fb_offset: i32, fdomain_offset: i32, output_offset: i32) -> i32);
        forward_call!(linker, store, "env", "crypto_secp256k1_verify", fn crypto_secp256k1_verify(digest: i32, digest_len: i32, sig: i32, sig_len: i32, recid: i32, pk_expected: i32, pk_expected_len: i32) -> i32);
    }

    pub fn catch_trap(err: RuntimeError) -> i32 {
        let err = match err {
            RuntimeError::Rwasm(err) => err,
            RuntimeError::ReducedModule(_) => return ExitCode::UnknownError as i32,
        };
        let err = match err {
            fluentbase_rwasm::Error::Trap(err) => err,
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
}
