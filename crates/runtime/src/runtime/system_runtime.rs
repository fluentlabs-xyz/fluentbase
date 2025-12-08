use crate::{syscall_handler::invoke_runtime_handler, RuntimeContext};
use fluentbase_types::{
    ExitCode, HashMap, RuntimeInterruptionOutcomeV1, SysFuncIdx, B256, STATE_DEPLOY, STATE_MAIN,
};
use rwasm::{ImportLinker, RwasmModule, TrapCode, ValType, Value, F32, F64, N_MAX_STACK_SIZE};
use smallvec::SmallVec;
use std::{
    cell::RefCell,
    mem::take,
    rc::Rc,
    sync::{Arc, OnceLock, RwLock},
};
use wasmtime::{
    AsContextMut, Config, Engine, Func, Instance, Linker, Memory, Module, OptLevel, Store,
    Strategy, Trap, Val,
};

pub struct SystemRuntime {
    // TODO(dmitry123): We can't have compiled runtime because it makes the runtime no `no_std` complaint,
    //  it should be fixed once we have optimized wasmtime inside rwasm repository.
    compiled_runtime: Rc<RefCell<CompiledRuntime>>,
    ctx: RuntimeContext,
    code_hash: B256,
    state: Option<RuntimeInterruptionOutcomeV1>,
}

struct CompiledRuntime {
    module: Module,
    store: Store<RuntimeContext>,
    instance: Instance,
    memory: Memory,
    deploy_func: Func,
    main_func: Func,
}

thread_local! {
    pub static COMPILED_RUNTIMES: RefCell<HashMap<B256, Rc<RefCell<CompiledRuntime>>>> = RefCell::new(HashMap::new());
}

impl SystemRuntime {
    pub fn compiled_module(code_hash: B256, rwasm_module: RwasmModule) -> Module {
        pub static COMPILED_MODULES: OnceLock<RwLock<HashMap<B256, Module>>> = OnceLock::new();
        let compiled_modules = COMPILED_MODULES.get_or_init(|| RwLock::new(HashMap::new()));
        {
            let guard = compiled_modules.read().unwrap();
            if let Some(module) = guard.get(&code_hash) {
                return module.clone();
            }
        }
        let mut guard = compiled_modules.write().unwrap();
        if let Some(module) = guard.get(&code_hash) {
            return module.clone();
        }
        let module = Module::new(wasmtime_engine(), &rwasm_module.hint_section).unwrap();
        guard.insert(code_hash, module.clone());
        module
    }

    pub fn reset_cached_runtimes() {
        COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
            compiled_runtimes.clear();
        });
    }

    pub fn new(
        module: RwasmModule,
        import_linker: Arc<ImportLinker>,
        code_hash: B256,
        ctx: RuntimeContext,
    ) -> Self {
        let compiled_runtime = COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
            if let Some(compiled_runtime) = compiled_runtimes.get(&code_hash).cloned() {
                return compiled_runtime;
            }
            let module = Self::compiled_module(code_hash, module);
            let engine = wasmtime_engine();
            let linker = wasmtime_import_linker(engine, import_linker);
            let mut store = Store::new(engine, RuntimeContext::default());
            let instance = linker.instantiate(store.as_context_mut(), &module).unwrap();
            let deploy_func = instance.get_func(store.as_context_mut(), "deploy").unwrap();
            let main_func = instance.get_func(store.as_context_mut(), "main").unwrap();
            let memory = instance
                .get_memory(store.as_context_mut(), "memory")
                .unwrap();
            let compiled_runtime = CompiledRuntime {
                module,
                store,
                instance,
                memory,
                deploy_func,
                main_func,
            };
            let compiled_runtime = Rc::new(RefCell::new(compiled_runtime));
            compiled_runtimes.insert(code_hash, compiled_runtime.clone());
            compiled_runtime
        });
        Self {
            compiled_runtime,
            ctx,
            code_hash,
            state: None,
        }
    }

    pub fn execute(&mut self) -> Result<(), TrapCode> {
        let mut compiled_runtime = self.compiled_runtime.borrow_mut();

        // Rewrite runtime context before each call, since we reuse the same store and runtime for
        // all EVM/SVM contract calls, then we should replace an existing context.
        //
        // SAFETY: We always call execute/resume in "right" order (w/o call overlying) that makes
        //  calls sequential and apps can't access non-their context.
        core::mem::swap(compiled_runtime.store.data_mut(), &mut self.ctx);

        // Call the function based on the passed state
        let entrypoint = match compiled_runtime.store.data().state {
            STATE_MAIN => compiled_runtime.main_func,
            STATE_DEPLOY => compiled_runtime.deploy_func,
            _ => unreachable!(),
        };
        let result = entrypoint
            .call(compiled_runtime.store.as_context_mut(), &[], &mut [])
            .map_err(map_anyhow_error);

        // Always swap back right after the call
        core::mem::swap(compiled_runtime.store.data_mut(), &mut self.ctx);

        if let Err(trap_code) = result.as_ref() {
            let exit_code = ExitCode::from(self.ctx.execution_result.exit_code);
            if exit_code == ExitCode::Panic {
                eprintln!(
                    "runtime: system execution failed with panic: {}, this should be investigated",
                    core::str::from_utf8(&self.ctx.execution_result.output)
                        .unwrap_or("can't decode utf-8 panic message")
                )
            } else if exit_code != ExitCode::Ok {
                eprintln!(
                    "runtime: system execution failed with exit code: {} ({}), this should be investigated",
                    exit_code, self.ctx.execution_result.exit_code
                )
            }
            eprintln!(
                "runtime: an unexpected trap code happened inside system runtime: {:?} ({}), falling back to the unreachable code, this should be investigated",
                trap_code, trap_code,
            );
            self.ctx.execution_result.exit_code =
                ExitCode::UnexpectedFatalExecutionFailure.into_i32();
            return Ok(());
        }

        // System runtime returns output with exit code (as first four bytes).
        // It doesn't halt, because halt or trap doesn't unwind the stack properly.
        let output = take(&mut self.ctx.execution_result.output);
        if output.len() < 4 {
            eprintln!(
                "runtime: an unexpected output size returned from system runtime: {}, falling back to the unreachable code, this should be investigated",
                output.len()
            );
            self.ctx.execution_result.exit_code =
                ExitCode::UnexpectedFatalExecutionFailure.into_i32();
            return Ok(());
        }
        let (exit_code_le, output) = output.split_at(4);
        self.ctx.execution_result.output = output.to_vec();
        let exit_code = i32::from_le_bytes(exit_code_le.try_into().unwrap());
        self.ctx.execution_result.exit_code = exit_code;

        // If the execution result is `InterruptionCalled`, then interruption is called, we should re-map
        // trap code into an interruption.
        //
        // SAFETY: Exit code `InterruptionCalled` can only be passed by our system runtimes, trustless
        //  applications can use this error code, but it won't be handled because of different
        //  runtime (only punishment for halt exit code).
        if ExitCode::from_repr(exit_code) == Some(ExitCode::InterruptionCalled) {
            // We need to move output into return data, because in our common case, interruptions
            // store syscall params inside return data,
            // but we can't suppose this for system runtime contracts because we don't expose such
            // functions, that's why we should move data from output into return data
            self.ctx.execution_result.return_data = take(&mut self.ctx.execution_result.output);
            assert!(
                !self.ctx.execution_result.return_data.is_empty(),
                "runtime: output can't be empty for interrupted call"
            );
            // Initialize resumable context with empty parameters, these values are passed into
            // the resume function once we're ready to resume
            self.state = Some(RuntimeInterruptionOutcomeV1::default());
            return Err(TrapCode::InterruptionCalled);
        }

        result
    }

    pub fn resume(&mut self, _exit_code: i32, _fuel_consumed: u64) -> Result<(), TrapCode> {
        // Make sure the runtime is always clear before resuming the call, because output is used
        // to pass interruption params in case of interruption
        self.ctx.clear_output();

        // Since we don't suppose native interruptions inside system runtimes then we just re-call
        // execute, but with passed return data with interruption outcome.
        //
        // Possible scenarios:
        // 1. w/ return data - new frame call
        // 2. w/o return data - current frame interruption outcome
        self.execute()
    }

    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        let mut compiled_runtime = self.compiled_runtime.borrow_mut();
        let memory = compiled_runtime.memory;
        memory
            .write(&mut compiled_runtime.store, offset, data)
            .map_err(|_| TrapCode::MemoryOutOfBounds)
    }

    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        let compiled_runtime = self.compiled_runtime.borrow();
        compiled_runtime
            .memory
            .read(&compiled_runtime.store, offset, buffer)
            .map_err(|_| TrapCode::MemoryOutOfBounds)
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        // We don't return the remaining fuel here because we don't know the remaining fuel,
        // also system runtime only used for trusted smart contracts with self gas management
        None
    }

    pub fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        func(&mut self.ctx)
    }

    pub fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        func(&self.ctx)
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
        cfg.cranelift_opt_level(OptLevel::Speed);
        cfg.parallel_compilation(true);
        cfg.consume_fuel(false);
        let engine = Engine::new(&cfg).unwrap();
        engine
    })
}

struct CallerAdapter<'a> {
    caller: wasmtime::Caller<'a, RuntimeContext>,
}

impl<'a> rwasm::Store<RuntimeContext> for CallerAdapter<'a> {
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

    fn try_consume_fuel(&mut self, _delta: u64) -> Result<(), TrapCode> {
        // There is no need to count this, we already have this counted inside the context
        Ok(())
    }

    fn remaining_fuel(&self) -> Option<u64> {
        let ctx = self.caller.data();
        Some(ctx.fuel_limit - ctx.execution_result.fuel_consumed)
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
    let mut caller_adapter = CallerAdapter::<'a> { caller };
    let sys_func_idx =
        SysFuncIdx::from_repr(sys_func_idx).ok_or(TrapCode::UnknownExternalFunction)?;
    let syscall_result = invoke_runtime_handler(
        &mut caller_adapter,
        sys_func_idx,
        mapped_params,
        mapped_result,
    );
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
        eprintln!("wasmtime trap code: {:?}", trap);
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
        eprintln!("wasmtime: unknown trap: {:?}", err);
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
