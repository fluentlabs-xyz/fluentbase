use crate::{syscall_handler::invoke_runtime_handler, RuntimeContext};
use fluentbase_types::{Address, HashMap, SysFuncIdx, STATE_DEPLOY, STATE_MAIN};
use rwasm::{
    ImportLinker, RwasmModule, Store, TrapCode, ValType, Value, F32, F64, N_MAX_STACK_SIZE,
};
use smallvec::SmallVec;
use std::{
    cell::RefCell,
    sync::{Arc, OnceLock},
};
use wasmtime::{
    AsContextMut, Config, Engine, Func, Instance, Linker, Module, OptLevel, Strategy, Trap, Val,
};

pub struct WasmtimeRuntime {
    compiled_runtime: Option<CompiledRuntime>,
    ctx: Option<RuntimeContext>,
    address: Address,
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
        }
    }

    pub fn execute(&mut self) -> Result<(), TrapCode> {
        let compiled_runtime = self.compiled_runtime.as_mut().unwrap();
        // Rewrite heap base on every execution to release already used memory
        compiled_runtime
            .heap_reset_func
            .call(compiled_runtime.store.as_context_mut(), &[], &mut [])
            .unwrap();
        // Rewrite runtime context before each call
        let ctx = self.ctx.take().unwrap();
        let entrypoint = match ctx.state {
            STATE_MAIN => compiled_runtime.main_func,
            STATE_DEPLOY => compiled_runtime.deploy_func,
            _ => unreachable!(),
        };
        *compiled_runtime.store.data_mut() = ctx;
        // Call the function based on the passed state
        let result = entrypoint.call(compiled_runtime.store.as_context_mut(), &[], &mut []);
        result.map_err(map_anyhow_error).or_else(|trap_code| {
            if trap_code == TrapCode::ExecutionHalted {
                Ok(())
            } else {
                Err(trap_code)
            }
        })
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

    pub fn resume(&mut self, _exit_code: i32) -> Result<(), TrapCode> {
        unreachable!()
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
    buffer.extend(std::iter::repeat(Value::I32(0)).take(result.len()));
    let (mapped_params, mapped_result) = buffer.split_at_mut(params.len());
    // caller adapter is required to provide operations for accessing memory and context
    let mut caller_adapter = CallerAdapter::<'a> {
        caller,
        fuel_consumed: 0,
    };
    let sys_func_idx =
        SysFuncIdx::from_repr(sys_func_idx).ok_or(TrapCode::UnknownExternalFunction)?;
    let syscall_result = invoke_runtime_handler(
        &mut caller_adapter,
        sys_func_idx,
        mapped_params,
        mapped_result,
    );
    match syscall_result {
        Err(TrapCode::InterruptionCalled) => {
            unreachable!("interruptions are not allowed")
        }
        _ => {}
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
        return Err(TrapCode::ExecutionHalted.into());
    }
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
            Trap::Interrupt => unreachable!("interrupt is not supported"),
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
