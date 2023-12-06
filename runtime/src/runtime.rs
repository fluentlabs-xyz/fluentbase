use crate::{
    fuel::*,
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
    IntoFunc,
    Linker,
    Module,
    StackLimits,
    Store,
};
use std::{cell::RefCell, mem::take};

#[derive(Debug)]
pub struct RuntimeContext<'t, T> {
    pub context: Option<&'t mut T>,
    // context inputs
    pub(crate) bytecode: Vec<u8>,
    pub(crate) fuel_limit: u32,
    pub(crate) state: u32,
    pub(crate) catch_trap: bool,
    pub(crate) input: Vec<u8>,
    // context outputs
    pub(crate) exit_code: i32,
    pub(crate) output: Vec<u8>,
}

impl<'t, T> Clone for RuntimeContext<'t, T> {
    fn clone(&self) -> Self {
        Self {
            context: None,
            bytecode: self.bytecode.clone(),
            fuel_limit: self.fuel_limit.clone(),
            state: self.state.clone(),
            catch_trap: self.catch_trap.clone(),
            input: self.input.clone(),
            exit_code: self.exit_code.clone(),
            output: self.output.clone(),
        }
    }
}

impl<'t, T> Default for RuntimeContext<'t, T> {
    fn default() -> Self {
        Self {
            context: None,
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

impl<'t, T> RuntimeContext<'t, T> {
    pub fn new<I: Into<Vec<u8>>>(bytecode: I) -> Self {
        Self {
            bytecode: bytecode.into(),
            ..Default::default()
        }
    }

    pub fn with_context(mut self, context: &'t mut T) -> Self {
        self.context = Some(context);
        self
    }

    pub fn with_input(mut self, input_data: Vec<u8>) -> Self {
        self.input = input_data;
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
}

#[derive(Debug)]
pub struct ExecutionResult<'t, T> {
    runtime_context: RuntimeContext<'t, T>,
    tracer: Tracer,
}

impl<'t, T> ExecutionResult<'t, T> {
    pub fn cloned(store: &Store<RuntimeContext<'t, T>>) -> Self {
        Self {
            runtime_context: store.data().clone(),
            tracer: store.tracer().clone(),
        }
    }

    pub fn taken(store: &mut Store<RuntimeContext<'t, T>>) -> Self {
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

    pub fn data(&self) -> &RuntimeContext<'t, T> {
        &self.runtime_context
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
    pub fn new_linker() -> ImportLinker {
        let mut import_linker = ImportLinker::default();
        // sys calls
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_halt".to_string(),
            SysFuncIdx::SYS_HALT as u16,
            FUEL_SYS_HALT,
            &[ValueType::I32; 1],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_state".to_string(),
            SysFuncIdx::SYS_STATE as u16,
            FUEL_SYS_STATE,
            &[],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_write".to_string(),
            SysFuncIdx::SYS_WRITE as u16,
            FUEL_SYS_WRITE,
            &[ValueType::I32; 2],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_sys_read".to_string(),
            SysFuncIdx::SYS_READ as u16,
            FUEL_SYS_READ,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        // WASI sys calls
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "proc_exit".to_string(),
            SysFuncIdx::WASI_PROC_EXIT as u16,
            FUEL_PROC_EXIT,
            &[ValueType::I32; 1],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "fd_write".to_string(),
            SysFuncIdx::WASI_FD_WRITE as u16,
            FUEL_FD_WRITE_FUEL,
            &[ValueType::I32; 4],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "environ_sizes_get".to_string(),
            SysFuncIdx::WASI_ENVIRON_SIZES_GET as u16,
            FUEL_ENVIRON_SIZES_GET,
            &[ValueType::I32; 2],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "environ_get".to_string(),
            SysFuncIdx::WASI_ENVIRON_GET as u16,
            FUEL_ENVIRON_GET,
            &[ValueType::I32; 2],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "args_sizes_get".to_string(),
            SysFuncIdx::WASI_ARGS_SIZES_GET as u16,
            FUEL_ARGS_SIZES_GET,
            &[ValueType::I32; 2],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "wasi_snapshot_preview1".to_string(),
            "args_get".to_string(),
            SysFuncIdx::WASI_ARGS_GET as u16,
            FUEL_ARGS_GET,
            &[ValueType::I32; 2],
            &[ValueType::I32; 1],
        ));
        // RWASM sys calls
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_rwasm_transact".to_string(),
            SysFuncIdx::RWASM_TRANSACT as u16,
            FUEL_RWASM_TRANSACT,
            &[ValueType::I32; 8],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_rwasm_compile".to_string(),
            SysFuncIdx::RWASM_COMPILE as u16,
            FUEL_RWASM_COMPILE,
            &[ValueType::I32; 4],
            &[ValueType::I32; 1],
        ));
        // zktrie
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_open".to_string(),
            SysFuncIdx::ZKTRIE_OPEN as u16,
            FUEL_ZKTRIE_OPEN,
            &[ValueType::I32; 0],
            &[ValueType::I32; 0],
        ));
        // account updates
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_update_nonce".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_NONCE as u16,
            FUEL_ZKTRIE_UPDATE_NONCE,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_update_balance".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_BALANCE as u16,
            FUEL_ZKTRIE_UPDATE_BALANCE,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_update_storage_root".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_STORAGE_ROOT as u16,
            FUEL_ZKTRIE_UPDATE_STORAGE_ROOT,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_update_code_hash".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_CODE_HASH as u16,
            FUEL_ZKTRIE_UPDATE_CODE_HASH,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_update_code_size".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_CODE_SIZE as u16,
            FUEL_ZKTRIE_UPDATE_CODE_SIZE,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        // account gets
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_get_nonce".to_string(),
            SysFuncIdx::ZKTRIE_GET_NONCE as u16,
            FUEL_ZKTRIE_GET_NONCE,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_get_balance".to_string(),
            SysFuncIdx::ZKTRIE_GET_BALANCE as u16,
            FUEL_ZKTRIE_GET_BALANCE,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_get_storage_root".to_string(),
            SysFuncIdx::ZKTRIE_GET_STORAGE_ROOT as u16,
            FUEL_ZKTRIE_GET_STORAGE_ROOT,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_get_code_hash".to_string(),
            SysFuncIdx::ZKTRIE_GET_CODE_HASH as u16,
            FUEL_ZKTRIE_GET_CODE_HASH,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_get_code_size".to_string(),
            SysFuncIdx::ZKTRIE_GET_CODE_SIZE as u16,
            FUEL_ZKTRIE_GET_CODE_SIZE,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        // store updates
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_update_store".to_string(),
            SysFuncIdx::ZKTRIE_UPDATE_STORE as u16,
            FUEL_ZKTRIE_UPDATE_STORE,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        // store gets
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_zktrie_get_store".to_string(),
            SysFuncIdx::ZKTRIE_GET_STORE as u16,
            FUEL_ZKTRIE_GET_STORE,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));

        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_mpt_open".to_string(),
            SysFuncIdx::MPT_OPEN as u16,
            FUEL_MPT_OPEN,
            &[ValueType::I32; 0],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_mpt_update".to_string(),
            SysFuncIdx::MPT_UPDATE as u16,
            FUEL_MPT_UPDATE,
            &[ValueType::I32; 4],
            &[ValueType::I32; 0],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_mpt_get".to_string(),
            SysFuncIdx::MPT_GET as u16,
            FUEL_MPT_GET,
            &[ValueType::I32; 3],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_mpt_get_root".to_string(),
            SysFuncIdx::MPT_GET_ROOT as u16,
            FUEL_MPT_GET_ROOT,
            &[ValueType::I32; 1],
            &[ValueType::I32; 1],
        ));
        // crypto
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_crypto_keccak256".to_string(),
            SysFuncIdx::CRYPTO_KECCAK256 as u16,
            FUEL_CRYPTO_KECCAK256,
            &[ValueType::I32; 3],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_crypto_poseidon".to_string(),
            SysFuncIdx::CRYPTO_POSEIDON as u16,
            FUEL_CRYPTO_POSEIDON,
            &[ValueType::I32; 3],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_crypto_poseidon2".to_string(),
            SysFuncIdx::CRYPTO_POSEIDON2 as u16,
            FUEL_CRYPTO_POSEIDON2,
            &[ValueType::I32; 4],
            &[],
        ));
        // ecc functions
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_ecc_secp256k1_verify".to_string(),
            SysFuncIdx::ECC_SECP256K1_VERIFY as u16,
            FUEL_ECC_SECP256K1_VERIFY,
            &[ValueType::I32; 7],
            &[ValueType::I32; 1],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_ecc_secp256k1_recover".to_string(),
            SysFuncIdx::ECC_SECP256K1_RECOVER as u16,
            FUEL_ECC_SECP256K1_RECOVER,
            &[ValueType::I32; 7],
            &[ValueType::I32; 1],
        ));
        // EVM
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_evm_sload".to_string(),
            SysFuncIdx::EVM_SLOAD as u16,
            FUEL_EVM_SLOAD,
            &[ValueType::I32; 2],
            &[],
        ));
        import_linker.insert_function(ImportFunc::new_env(
            "env".to_string(),
            "_evm_sstore".to_string(),
            SysFuncIdx::EVM_SSTORE as u16,
            FUEL_EVM_SSTORE,
            &[ValueType::I32; 2],
            &[],
        ));
        import_linker
    }

    #[deprecated(note = "use `run_with_context` instead")]
    pub fn run(
        rwasm_binary: &[u8],
        input_data: &Vec<u8>,
        fuel_limit: u32,
    ) -> Result<ExecutionResult<'t, T>, RuntimeError> {
        let runtime_context = RuntimeContext::new(rwasm_binary)
            .with_input(input_data.clone())
            .with_catch_trap(true)
            .with_fuel_limit(fuel_limit);
        let import_linker = Self::new_linker();
        Self::run_with_context(runtime_context, &import_linker)
    }

    pub fn run_with_context(
        mut runtime_context: RuntimeContext<'t, T>,
        import_linker: &ImportLinker,
    ) -> Result<ExecutionResult<'t, T>, RuntimeError> {
        let catch_error = runtime_context.catch_trap;
        let runtime = Self::new(runtime_context.clone(), import_linker);
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
            let reduced_module = ReducedModule::new(runtime_context.bytecode.as_slice(), false)
                .map_err(Into::<RuntimeError>::into)?;
            let module_builder =
                reduced_module.to_module_builder(&engine, import_linker, FuncType::new([], []));
            (module_builder.finish(), reduced_module.bytecode().clone())
        };

        let mut linker = Linker::<RuntimeContext<T>>::new(&engine);
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
        let linker = &mut self.linker;
        let store = &mut self.store;
        use crate::instruction::*;
        // sys
        forward_call!(linker, store, "env", "_sys_halt", fn sys_halt(exit_code: u32) -> ());
        forward_call!(linker, store, "env", "_sys_state", fn sys_state() -> u32);
        forward_call!(linker, store, "env", "_sys_read", fn sys_read(target: u32, offset: u32, length: u32) -> u32);
        forward_call!(linker, store, "env", "_sys_write", fn sys_write(offset: u32, length: u32) -> ());
        // wasi
        forward_call!(linker, store, "wasi_snapshot_preview1", "proc_exit", fn wasi_proc_exit(exit_code: i32) -> ());
        forward_call!(linker, store, "wasi_snapshot_preview1", "fd_write", fn wasi_fd_write(fd: i32, iovs_ptr: i32, iovs_len: i32, rp0_ptr: i32) -> i32);
        forward_call!(linker, store, "wasi_snapshot_preview1", "environ_sizes_get", fn wasi_environ_sizes_get(rp0_ptr: i32, rp1_ptr: i32) -> i32);
        forward_call!(linker, store, "wasi_snapshot_preview1", "environ_get", fn wasi_environ_get(environ: i32, environ_buffer: i32) -> i32);
        forward_call!(linker, store, "wasi_snapshot_preview1", "args_sizes_get", fn wasi_args_sizes_get(argc_ptr: i32, argv_ptr: i32) -> i32);
        forward_call!(linker, store, "wasi_snapshot_preview1", "args_get", fn wasi_args_get(argv_ptrs_ptr: i32, argv_buff_ptr: i32) -> i32);
        // rwasm
        forward_call!(linker, store, "env", "_rwasm_compile", fn rwasm_compile(input_ptr: u32, input_len: u32, output_ptr: u32, output_len: u32) -> i32);
        forward_call!(linker, store, "env", "_rwasm_transact", fn rwasm_transact(code_offset: i32, code_len: i32, input_offset: i32, input_len: i32, output_offset: i32, output_len: i32, state: i32, fuel_limit: i32) -> i32);
        // zktrie
        // forward_call!(linker, store, "env", "_zktrie_open", fn zktrie_open() -> ());
        // forward_call!(linker, store, "env", "_zktrie_update_nonce", fn
        // zktrie_update_nonce(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) ->
        // ()); forward_call!(linker, store, "env", "_zktrie_get_nonce", fn
        // zktrie_get_nonce(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        // forward_call!(linker, store, "env", "_zktrie_update_balance", fn
        // zktrie_update_balance(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32)
        // -> ()); forward_call!(linker, store, "env", "_zktrie_get_balance", fn
        // zktrie_get_balance(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        // forward_call!(linker, store, "env", "_zktrie_update_storage_root", fn
        // zktrie_update_storage_root(key_offset: i32, key_len: i32, value_offset: i32, value_len:
        // i32) -> ()); forward_call!(linker, store, "env", "_zktrie_get_storage_root", fn
        // zktrie_get_storage_root(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        // forward_call!(linker, store, "env", "_zktrie_update_code_hash", fn
        // zktrie_update_code_hash(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32)
        // -> ()); forward_call!(linker, store, "env", "_zktrie_get_code_hash", fn
        // zktrie_get_code_hash(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        // forward_call!(linker, store, "env", "_zktrie_update_code_size", fn
        // zktrie_update_code_size(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32)
        // -> ()); forward_call!(linker, store, "env", "_zktrie_get_code_size", fn
        // zktrie_get_code_size(key_offset: i32, key_len: i32, output_offset: i32) -> i32);
        // forward_call!(linker, store, "env", "_zktrie_update_store", fn
        // zktrie_update_store(key_offset: i32, key_len: i32, value_offset: i32, value_len: i32) ->
        // ()); forward_call!(linker, store, "env", "_zktrie_get_store", fn
        // zktrie_get_store(key_offset: i32, key_len: i32, output_offset: i32) -> i32); mpt
        // forward_call!(linker, store, "env", "_mpt_open", fn mpt_open() -> ());
        // forward_call!(linker, store, "env", "_mpt_update", fn mpt_update(key_offset: i32,
        // key_len: i32, value_offset: i32, value_len: i32) -> ()); forward_call!(linker,
        // store, "env", "_mpt_get", fn mpt_get(key_offset: i32, key_len: i32, output_offset: i32)
        // -> i32); forward_call!(linker, store, "env", "_mpt_get_root", fn
        // mpt_get_root(output_offset: i32) -> i32); crypto
        forward_call!(linker, store, "env", "_crypto_keccak256", fn crypto_keccak256(data_offset: i32, data_len: i32, output_offset: i32) -> ());
        forward_call!(linker, store, "env", "_crypto_poseidon", fn crypto_poseidon(data_offset: i32, data_len: i32, output_offset: i32) -> ());
        forward_call!(linker, store, "env", "_crypto_poseidon2", fn crypto_poseidon2(fa_offset: i32, fb_offset: i32, fdomain_offset: i32, output_offset: i32) -> ());
        // ecc
        forward_call!(linker, store, "env", "_ecc_secp256k1_verify", fn ecc_secp256k1_verify(digest: i32, digest_len: i32, sig: i32, sig_len: i32, pk_expected: i32, pk_expected_len: i32, rec_id: i32) -> i32);
        forward_call!(linker, store, "env", "_ecc_secp256k1_recover", fn ecc_secp256k1_recover(signature: i32, signature_len: i32, digest: i32, digest_len: i32, output: i32, output_len: i32, rec_id: i32) -> i32);
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
