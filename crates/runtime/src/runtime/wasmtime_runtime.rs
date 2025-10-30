use crate::syscall_handler::InterruptionHolder;
use crate::{syscall_handler::invoke_runtime_handler, RuntimeContext};
use fluentbase_types::int_state::{
    bincode_encode, bincode_encode_prefixed, bincode_try_decode, bincode_try_decode_prefixed,
    IntState, INT_PREFIX,
};
use fluentbase_types::{
    log_ext, Address, ExitCode, HashMap, SysFuncIdx, SyscallInvocationParams, STATE_DEPLOY,
    STATE_MAIN,
};
use rwasm::{
    ImportLinker, RwasmModule, Store, TrapCode, ValType, Value, F32, F64, N_MAX_STACK_SIZE,
};
use smallvec::SmallVec;
use std::mem::take;
use std::{
    cell::RefCell,
    sync::{Arc, OnceLock},
};
use wasmtime::{
    AsContext, AsContextMut, Config, Engine, Func, Instance, Linker, Module, OptLevel, Strategy,
    Trap, Val,
};

pub struct WasmtimeRuntime {
    compiled_runtime: Option<CompiledRuntime>,
    ctx: Option<RuntimeContext>,
    address: Address,
    int_state: Option<IntState>,
}

struct CompiledRuntime {
    module: Module,
    store: wasmtime::Store<RuntimeContext>,
    instance: Instance,
    deploy_func: Func,
    main_func: Func,
    heap_reset_func: Func,
}

thread_local! {
    pub static COMPILED_RUNTIMES: RefCell<HashMap<Address, CompiledRuntime>> = RefCell::new(HashMap::new());
}

impl Drop for WasmtimeRuntime {
    fn drop(&mut self) {
        COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
            compiled_runtimes.insert(self.address, self.compiled_runtime.take().unwrap());
        });
    }
}

impl WasmtimeRuntime {
    pub fn compile_module(rwasm_module: RwasmModule) -> Module {
        Module::new(wasmtime_engine(), &rwasm_module.hint_section).unwrap()
    }

    pub fn new(
        module: RwasmModule,
        import_linker: Arc<ImportLinker>,
        address: Address,
        ctx: RuntimeContext,
    ) -> Self {
        let compiled_runtime = COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
            if let Some(compiled_runtime) = compiled_runtimes.remove(&address) {
                return compiled_runtime;
            }
            let module = Self::compile_module(module);
            let engine = wasmtime_engine();
            let linker = wasmtime_import_linker(engine, import_linker);
            let mut store = wasmtime::Store::new(engine, RuntimeContext::default());
            let instance = linker.instantiate(store.as_context_mut(), &module).unwrap();
            let deploy_func = instance.get_func(store.as_context_mut(), "deploy").unwrap();
            let main_func = instance.get_func(store.as_context_mut(), "main").unwrap();
            let heap_reset_func = instance
                .get_func(store.as_context_mut(), "__heap_reset")
                .unwrap();
            CompiledRuntime {
                module,
                store,
                instance,
                deploy_func,
                main_func,
                heap_reset_func,
            }
        });
        Self {
            compiled_runtime: Some(compiled_runtime),
            ctx: Some(ctx),
            address,
            int_state: None,
        }
    }

    pub fn execute(&mut self) -> Result<(), TrapCode> {
        let compiled_runtime = self.compiled_runtime.as_mut().unwrap();
        // Rewrite runtime context before each call
        if self.ctx.is_some() {
            // Rewrite heap base on every execution to release already used memory
            compiled_runtime
                .heap_reset_func
                .call(compiled_runtime.store.as_context_mut(), &[], &mut [])
                .unwrap();
            let ctx = self.ctx.take().unwrap();
            *compiled_runtime.store.data_mut() = ctx;
        }
        let entrypoint = match compiled_runtime.store.data().state {
            STATE_MAIN => compiled_runtime.main_func,
            STATE_DEPLOY => compiled_runtime.deploy_func,
            _ => unreachable!(),
        };
        // let mut int_state: Option<IntState> = None;
        // let mut depth = 0;
        // let result = loop {
        // if let Some(int_state) = int_state.take() {
        //     log_ext!();
        //     let int_state_encoded = bincode_encode_prefixed(INT_PREFIX, &int_state);
        //     log_ext!("int_state_encoded.len={}", int_state_encoded.len());
        //     compiled_runtime.store.as_context_mut().data_mut().input = int_state_encoded.into();
        // }
        // Call the function based on the passed state
        let result = entrypoint.call(compiled_runtime.store.as_context_mut(), &[], &mut []);
        let mut runtime_ctx = compiled_runtime.store.as_context_mut();
        // log_ext!("depth={}", depth);
        if runtime_ctx.data().execution_result.exit_code == ExitCode::InterruptionCalled.into_i32()
        {
            // let int_state_decoded: IntState =
            //     bincode_try_decode(&runtime_ctx.data().execution_result.output).unwrap();
            // let syscall_params =
            //     SyscallInvocationParams::decode(&int_state_decoded.syscall_params).unwrap();
            // runtime_ctx.data_mut().resumable_context = Some(InterruptionHolder {
            //     params: syscall_params,
            //     is_root: false,
            // });
            // runtime_ctx.data_mut().execution_result.int_state = Some(int_state_decoded);
            self.int_state = runtime_ctx.data_mut().execution_result.int_state.take();
            return Err(TrapCode::InterruptionCalled);
        } // else if depth > 0 {
          //     depth -= 1;
          //     // TODO handle recovery after interruption
          //     continue;
          // }
          // break result;
          // };
        result.map_err(map_anyhow_error).or_else(|trap_code| {
            if trap_code == TrapCode::ExecutionHalted {
                Ok(())
            } else {
                log_ext!("trap_code={}", trap_code);
                Err(trap_code)
            }
        })
    }

    pub fn resume(&mut self, exit_code: i32) -> Result<(), TrapCode> {
        log_ext!("exit code={}", exit_code);
        if let Some(int_state) = self.int_state.take() {
            let mut store_mut = self
                .compiled_runtime
                .as_mut()
                .unwrap()
                .store
                .as_context_mut();
            let data_mut = store_mut.data_mut();
            let return_data = take(&mut data_mut.execution_result.return_data);
            data_mut.input = bincode_encode_prefixed(INT_PREFIX, &int_state).into();
            data_mut.execution_result = Default::default();
            data_mut.execution_result.return_data = return_data;
            let result = self.execute();
            return result;
        }
        // unreachable!()
        Ok(())
    }

    pub fn try_consume_fuel(&mut self, _fuel: u64) -> Result<(), TrapCode> {
        // We don't support fuel for this runtime
        Ok(())
    }

    pub fn memory_write(&mut self, _offset: usize, _data: &[u8]) -> Result<(), TrapCode> {
        unimplemented!()
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        // We don't support fuel for this runtime
        None
    }

    pub fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        let compiled_runtime = self.compiled_runtime.as_mut().unwrap();
        func(compiled_runtime.store.data_mut())
    }

    pub fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        let compiled_runtime = self.compiled_runtime.as_ref().unwrap();
        func(compiled_runtime.store.data())
    }
}

fn wasmtime_engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut cfg = Config::new();
        cfg.strategy(Strategy::Cranelift);
        cfg.collector(wasmtime::Collector::Null);
        cfg.max_wasm_stack(N_MAX_STACK_SIZE * size_of::<u32>());
        cfg.async_support(false);
        cfg.wasm_memory64(false);
        cfg.memory_init_cow(false);
        cfg.cranelift_opt_level(OptLevel::SpeedAndSize);
        cfg.parallel_compilation(true);
        cfg.consume_fuel(false);
        let engine = Engine::new(&cfg).unwrap();
        engine
    })
}

struct CallerAdapter<'a> {
    caller: wasmtime::Caller<'a, RuntimeContext>,
    fuel_consumed: u64,
}

impl<'a> Store<RuntimeContext> for CallerAdapter<'a> {
    fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        let memory = self
            .caller
            .get_export("memory")
            .unwrap()
            .into_memory()
            .unwrap();
        memory
            .read(self.caller.as_context_mut(), offset, buffer)
            .map_err(|_| TrapCode::MemoryOutOfBounds)
    }

    fn memory_write(&mut self, offset: usize, buffer: &[u8]) -> Result<(), TrapCode> {
        let memory = self
            .caller
            .get_export("memory")
            .unwrap()
            .into_memory()
            .unwrap();
        memory
            .write(self.caller.as_context_mut(), offset, buffer)
            .map_err(|_| TrapCode::MemoryOutOfBounds)
    }

    fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        func(self.caller.data_mut())
    }

    fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        func(self.caller.data())
    }

    fn try_consume_fuel(&mut self, delta: u64) -> Result<(), TrapCode> {
        self.fuel_consumed += delta;
        Ok(())
    }

    fn remaining_fuel(&self) -> Option<u64> {
        None
    }
}

fn wasmtime_import_linker(
    engine: &Engine,
    import_linker: Arc<ImportLinker>,
) -> Linker<RuntimeContext> {
    let mut linker = Linker::<RuntimeContext>::new(engine);
    for (import_name, import_entity) in import_linker.iter() {
        let params = import_entity
            .params
            .iter()
            .copied()
            .map(map_val_type)
            .collect::<Vec<_>>();
        let result = import_entity
            .result
            .iter()
            .copied()
            .map(map_val_type)
            .collect::<Vec<_>>();
        let func_type = wasmtime::FuncType::new(engine, params, result);
        linker
            .func_new(
                import_name.module(),
                import_name.name(),
                func_type,
                move |caller, params, result| {
                    wasmtime_syscall_handler(import_entity.sys_func_idx, caller, params, result)
                },
            )
            .unwrap_or_else(|_| panic!("function import collision: {}", import_name));
    }
    linker
}

fn wasmtime_syscall_handler<'a>(
    sys_func_idx: u32,
    caller: wasmtime::Caller<'a, RuntimeContext>,
    params: &[Val],
    result: &mut [Val],
) -> anyhow::Result<()> {
    // convert input values from wasmtime format into rwasm format
    let mut buffer = SmallVec::<[Value; 32]>::new();
    buffer.extend(params.iter().map(|x| match x {
        Val::I32(value) => Value::I32(*value),
        Val::I64(value) => Value::I64(*value),
        Val::F32(value) => Value::F32(F32::from_bits(*value)),
        Val::F64(value) => Value::F64(F64::from_bits(*value)),
        _ => unreachable!("wasmtime: not supported type: {:?}", x),
    }));
    buffer.extend(core::iter::repeat(Value::I32(0)).take(result.len()));
    let (mapped_params, mapped_result) = buffer.split_at_mut(params.len());
    // caller adapter is required to provide operations for accessing memory and context
    let mut caller_adapter = CallerAdapter::<'a> {
        caller,
        fuel_consumed: 0,
    };
    let sys_func_idx =
        SysFuncIdx::from_repr(sys_func_idx).ok_or(TrapCode::UnknownExternalFunction)?;
    log_ext!("sys_func_idx: {:?}", sys_func_idx);
    let syscall_result = invoke_runtime_handler(
        &mut caller_adapter,
        sys_func_idx,
        mapped_params,
        mapped_result,
    );
    let execution_result = &mut caller_adapter.caller.data_mut().execution_result;
    log_ext!(
        "caller_adapter.caller.data_mut().execution_result.output({})={:x?} exit_code={:?} mapped_params={:?} mapped_result={:?}",
        execution_result.output.len(),
        execution_result.output,
        execution_result.exit_code,
        mapped_params,
        mapped_result,
    );
    if execution_result.exit_code == ExitCode::InterruptionCalled.into_i32() {
        let int_state = bincode_try_decode::<IntState>(&execution_result.output)
            .expect("output contains interruption state");
        let syscall_params = SyscallInvocationParams::decode(&int_state.syscall_params).unwrap();
        caller_adapter.caller.data_mut().resumable_context = Some(InterruptionHolder {
            params: syscall_params,
            is_root: false,
        });
        caller_adapter.caller.data_mut().execution_result.int_state = Some(int_state);
        return Ok(());
    }
    // make sure a syscall result is successful
    let should_terminate = syscall_result.map(|_| false).or_else(|trap_code| {
        // if syscall returns execution halted,
        // then don't return this trap code since it's a successful error code
        if trap_code == TrapCode::ExecutionHalted {
            Ok(true)
        } else {
            Err(trap_code)
        }
    })?;
    log_ext!();
    // after call map all values back to wasmtime format
    for (i, value) in mapped_result.iter().enumerate() {
        result[i] = match value {
            Value::I32(value) => Val::I32(*value),
            Value::I64(value) => Val::I64(*value),
            Value::F32(value) => Val::F32(value.to_bits()),
            Value::F64(value) => Val::F64(value.to_bits()),
            _ => unreachable!("wasmtime: not supported type: {:?}", value),
        };
    }
    // terminate execution if required
    if should_terminate {
        log_ext!();
        return Err(TrapCode::ExecutionHalted.into());
    }
    log_ext!();
    Ok(())
}

fn map_anyhow_error(err: anyhow::Error) -> TrapCode {
    if let Some(trap) = err.downcast_ref::<Trap>() {
        // map wasmtime trap codes into our trap codes
        use wasmtime::Trap;
        match trap {
            Trap::StackOverflow => TrapCode::StackOverflow,
            Trap::MemoryOutOfBounds => TrapCode::MemoryOutOfBounds,
            Trap::HeapMisaligned => TrapCode::MemoryOutOfBounds,
            Trap::TableOutOfBounds => TrapCode::TableOutOfBounds,
            Trap::IndirectCallToNull => TrapCode::IndirectCallToNull,
            Trap::BadSignature => TrapCode::BadSignature,
            Trap::IntegerOverflow => TrapCode::IntegerOverflow,
            Trap::IntegerDivisionByZero => TrapCode::IntegerDivisionByZero,
            Trap::BadConversionToInteger => TrapCode::BadConversionToInteger,
            Trap::UnreachableCodeReached => TrapCode::UnreachableCodeReached,
            Trap::Interrupt => TrapCode::InterruptionCalled,
            Trap::AlwaysTrapAdapter => unreachable!("component-model is not supported"),
            Trap::OutOfFuel => TrapCode::OutOfFuel,
            Trap::AtomicWaitNonSharedMemory => {
                unreachable!("atomic extension is not supported")
            }
            Trap::NullReference => TrapCode::IndirectCallToNull,
            Trap::ArrayOutOfBounds | Trap::AllocationTooLarge => {
                unreachable!("gc is not supported")
            }
            Trap::CastFailure => TrapCode::BadConversionToInteger,
            Trap::CannotEnterComponent => unreachable!("component-model is not supported"),
            Trap::NoAsyncResult => unreachable!("async mode must be disabled"),
            _ => unreachable!("unknown trap wasmtime code"),
        }
    } else if let Some(trap) = err.downcast_ref::<TrapCode>() {
        // if our trap code is initiated, then just return the trap code
        *trap
    } else {
        eprintln!("wasmtime unknown trap: {:?}", err);
        // TODO(dmitry123): "what type of error to use here in case of unknown error?"
        TrapCode::IllegalOpcode
    }
}

fn map_val_type(val_type: ValType) -> wasmtime::ValType {
    match val_type {
        ValType::I32 => wasmtime::ValType::I32,
        ValType::I64 => wasmtime::ValType::I64,
        ValType::F32 => wasmtime::ValType::F32,
        ValType::F64 => wasmtime::ValType::F64,
        _ => unreachable!("wasmtime: not supported type: {:?}", val_type),
    }
}
