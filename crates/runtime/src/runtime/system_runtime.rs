use crate::{syscall_handler::invoke_runtime_handler, RuntimeContext};
use fluentbase_types::{
    bincode, log_ext, ExitCode, HashMap, RuntimeInterruptionOutcomeV1, SysFuncIdx, B256,
    STATE_DEPLOY, STATE_MAIN,
};
use num::ToPrimitive;
#[cfg(not(feature = "std"))]
use revm_helpers::reusable_pool::global::VecU8;
use revm_helpers::reusable_pool::global::VecU8;
use rwasm::{ImportLinker, RwasmModule, TrapCode, ValType, Value, F32, F64, N_MAX_STACK_SIZE};
use smallvec::SmallVec;
use std::{
    cell::RefCell,
    mem::take,
    sync::{Arc, OnceLock, RwLock},
};
use wasmtime::{
    AsContextMut, Config, Engine, Func, Instance, Linker, Memory, Module, OptLevel, Store,
    Strategy, Trap, Val,
};

pub struct SystemRuntime {
    // TODO(dmitry123): We can't have compiled runtime because it makes the runtime no `no_std` complaint,
    //  it should be fixed once we have optimized wasmtime inside rwasm repository.
    compiled_runtime: Option<CompiledRuntime>,
    ctx: Option<RuntimeContext>,
    code_hash: B256,
    state: Option<RuntimeInterruptionOutcomeV1>,
    depth: i32,
    exec_count: i32,
    import_linker: Arc<ImportLinker>,
}

pub struct CompiledRuntime {
    module: Module,
    store: Store<RuntimeContext>,
    instance: Instance,
    memory: Memory,
    deploy_func: Func,
    main_func: Func,
    heap_pos_func: Func,
    heap_pos_set_func: Func,
    heap_reset_func: Func,
    heap_base_offset_func: Func,
    alloc_count_func: Func,
    alloc_bytes_func: Func,
    dealloc_try_count_func: Func,
    dealloc_try_bytes_func: Func,
    dealloc_count_func: Func,
    dealloc_bytes_func: Func,
}

impl CompiledRuntime {
    pub fn heap_base_offset(&mut self) -> Option<u32> {
        log_ext!();
        let result = &mut [Val::I32(0)];
        self.heap_base_offset_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        Some(result[0].i32().unwrap().to_u32().unwrap())
    }

    pub fn alloc_stats(&mut self) -> (u32, u32, u32, u32, u32, u32) {
        log_ext!();
        let result = &mut [Val::I32(0)];
        self.alloc_count_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        let alloc_count = result[0].i32().unwrap().to_u32().unwrap();
        self.alloc_bytes_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        let alloc_bytes = result[0].i32().unwrap().to_u32().unwrap();
        self.dealloc_try_count_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        let count_try = result[0].i32().unwrap().to_u32().unwrap();
        self.dealloc_try_bytes_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        let bytes_try = result[0].i32().unwrap().to_u32().unwrap();
        self.dealloc_count_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        let dealloc_count = result[0].i32().unwrap().to_u32().unwrap();
        self.dealloc_bytes_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        let dealloc_bytes = result[0].i32().unwrap().to_u32().unwrap();
        (
            alloc_count,
            alloc_bytes,
            count_try,
            bytes_try,
            dealloc_count,
            dealloc_bytes,
        )
    }

    pub fn heap_pos(&mut self) -> u32 {
        log_ext!();
        let result = &mut [Val::I32(0)];
        self.heap_pos_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        result[0].i32().unwrap().to_u32().unwrap()
    }

    pub fn heap_pos_set(&mut self, value: u32) {
        log_ext!();
        let params = &[Val::I32(value as i32)];
        let results = &mut [];
        self.heap_pos_func
            .call(self.store.as_context_mut(), params, results)
            .unwrap();
    }

    pub fn heap_reset(&mut self) {
        log_ext!();
        log_ext!("heap pos before reset {:?}", self.heap_pos());
        let result = &mut [Val::I32(0)];
        self.heap_reset_func
            .call(self.store.as_context_mut(), &[], result)
            .unwrap();
        log_ext!(
            "heap pos after reset {} ({:?})",
            result[0].i32().unwrap().to_i32().unwrap(),
            self.heap_pos()
        );

        // let memory: Memory = self
        //     .instance
        //     .get_memory(self.store.as_context_mut(), "memory")
        //     .unwrap();
        // self.memory = memory;
    }
}

thread_local! {
    pub static COMPILED_RUNTIMES: RefCell<HashMap<B256, CompiledRuntime>> = RefCell::new(HashMap::new());
}

impl Drop for SystemRuntime {
    fn drop(&mut self) {
        let _ = COMPILED_RUNTIMES.try_with(|compiled_runtimes| {
            log_ext!();
            let compiled_runtime: CompiledRuntime = self.compiled_runtime.take().unwrap();
            compiled_runtimes
                .borrow_mut()
                .insert(self.code_hash, compiled_runtime);
        });
    }
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

    pub fn new(
        module: RwasmModule,
        import_linker: Arc<ImportLinker>,
        code_hash: B256,
        ctx: RuntimeContext,
    ) -> Self {
        let compiled_runtime = COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
            if let Some(compiled_runtime) = compiled_runtimes.remove(&code_hash) {
                return compiled_runtime;
            }
            let module = Self::compiled_module(code_hash, module);
            let engine = wasmtime_engine();
            let linker = wasmtime_import_linker(engine, import_linker.clone());
            let mut store = Store::new(engine, RuntimeContext::default());
            let instance = linker.instantiate(store.as_context_mut(), &module).unwrap();
            let deploy_func = instance.get_func(store.as_context_mut(), "deploy").unwrap();
            let main_func = instance.get_func(store.as_context_mut(), "main").unwrap();
            let memory = instance
                .get_memory(store.as_context_mut(), "memory")
                .unwrap();

            let heap_pos_func = instance
                .get_func(store.as_context_mut(), "__heap_pos")
                .unwrap();
            let heap_pos_set_func = instance
                .get_func(store.as_context_mut(), "__heap_pos_set")
                .unwrap();
            let heap_reset_func = instance
                .get_func(store.as_context_mut(), "__heap_reset")
                .unwrap();
            let heap_base_offset_func = instance
                .get_func(store.as_context_mut(), "__heap_base_offset")
                .unwrap();
            let alloc_count_func = instance
                .get_func(store.as_context_mut(), "__alloc_count")
                .unwrap();
            let alloc_bytes_func = instance
                .get_func(store.as_context_mut(), "__alloc_bytes")
                .unwrap();
            let dealloc_try_count_func = instance
                .get_func(store.as_context_mut(), "__dealloc_try_count")
                .unwrap();
            let dealloc_try_bytes_func = instance
                .get_func(store.as_context_mut(), "__dealloc_try_bytes")
                .unwrap();
            let dealloc_count_func = instance
                .get_func(store.as_context_mut(), "__dealloc_count")
                .unwrap();
            let dealloc_bytes_func = instance
                .get_func(store.as_context_mut(), "__dealloc_bytes")
                .unwrap();

            CompiledRuntime {
                module,
                store,
                instance,
                memory,
                deploy_func,
                main_func,
                heap_pos_func,
                heap_pos_set_func,
                heap_reset_func,
                heap_base_offset_func,
                alloc_count_func,
                alloc_bytes_func,
                dealloc_try_count_func,
                dealloc_try_bytes_func,
                dealloc_count_func,
                dealloc_bytes_func,
            }
        });
        Self {
            compiled_runtime: Some(compiled_runtime),
            ctx: Some(ctx),
            code_hash,
            state: None,
            depth: 0,
            exec_count: 0,
            import_linker: import_linker.clone(),
        }
    }

    fn reset_compiled_runtime(
        compiled_runtime: &mut CompiledRuntime,
        import_linker: Arc<ImportLinker>,
    ) {
        compiled_runtime.heap_reset();
        let engine = wasmtime_engine();
        let linker = wasmtime_import_linker(engine, import_linker);
        let instance = linker
            .instantiate(
                compiled_runtime.store.as_context_mut(),
                &compiled_runtime.module,
            )
            .unwrap();
        let deploy_func = instance
            .get_func(compiled_runtime.store.as_context_mut(), "deploy")
            .unwrap();
        let main_func = instance
            .get_func(compiled_runtime.store.as_context_mut(), "main")
            .unwrap();
        let memory = instance
            .get_memory(compiled_runtime.store.as_context_mut(), "memory")
            .unwrap();

        let heap_pos_func = instance
            .get_func(compiled_runtime.store.as_context_mut(), "__heap_pos")
            .unwrap();
        let heap_pos_set_func = instance
            .get_func(compiled_runtime.store.as_context_mut(), "__heap_pos_set")
            .unwrap();
        let heap_reset_func = instance
            .get_func(compiled_runtime.store.as_context_mut(), "__heap_reset")
            .unwrap();
        let heap_base_offset_func = instance
            .get_func(
                compiled_runtime.store.as_context_mut(),
                "__heap_base_offset",
            )
            .unwrap();

        // compiled_runtime.module = module;
        // compiled_runtime.store = store;
        compiled_runtime.instance = instance;
        compiled_runtime.memory = memory;

        compiled_runtime.deploy_func = deploy_func;
        compiled_runtime.main_func = main_func;

        compiled_runtime.heap_pos_func = heap_pos_func;
        compiled_runtime.heap_pos_set_func = heap_pos_set_func;
        compiled_runtime.heap_reset_func = heap_reset_func;
        compiled_runtime.heap_base_offset_func = heap_base_offset_func;
    }

    pub fn execute(&mut self) -> Result<(), TrapCode> {
        let compiled_runtime = self.compiled_runtime.as_mut().unwrap();

        // Rewrite runtime context before each call, since we reuse the same store and runtime for
        // all EVM/SVM contract calls, then we should replace an existing context.
        //
        // SAFETY: We always call execute/resume in "right" order (w/o call overlying) that makes
        //  calls sequential and apps can't access non-their context.
        if let Some(ctx) = self.ctx.take() {
            *compiled_runtime.store.data_mut() = ctx;
        }

        // Call the function based on the passed state
        let state = compiled_runtime.store.data().state;
        let entrypoint = match state {
            STATE_MAIN => compiled_runtime.main_func,
            STATE_DEPLOY => compiled_runtime.deploy_func,
            _ => unreachable!(),
        };
        let result = entrypoint.call(compiled_runtime.store.as_context_mut(), &[], &mut []);

        // System runtime returns output with exit code (as first four bytes).
        // It doesn't halt, because halt or trap doesn't unwind the stack properly.
        let ctx = compiled_runtime.store.data_mut();
        let output = take(&mut ctx.execution_result.output);
        if output.len() < 4 {
            eprintln!(
                "runtime: an unexpected output size returned from system runtime: {}, falling back to the unreachable code, this should be investigated",
                output.len()
            );
            return Err(TrapCode::UnreachableCodeReached);
        }
        let (exit_code_le, output) = output.split_at(4);
        ctx.execution_result.output = output.to_vec();
        let exit_code = i32::from_le_bytes(exit_code_le.try_into().unwrap());
        ctx.execution_result.exit_code = exit_code;

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
            ctx.execution_result.return_data = take(&mut ctx.execution_result.output);
            // Initialize resumable context with empty parameters, these values are passed into
            // the resume function once we're ready to resume
            self.state = Some(RuntimeInterruptionOutcomeV1::default());
            return Err(TrapCode::InterruptionCalled);
        }

        let result = result.map_err(map_anyhow_error).or_else(|trap_code| {
            // Trap code `ExecutionHalted` is used to unwind the execution and terminate, that's
            // why we map it into `Ok()`
            log_ext!("trap_code={:?}", trap_code);
            if trap_code == TrapCode::ExecutionHalted {
                Ok(())
            } else {
                Err(trap_code)
            }
        });

        if let Err(trap_code) = &result {
            eprintln!(
                "runtime: an unexpected trap code happened inside system runtime: {}, falling back to the unreachable code, this should be investigated",
                trap_code
            );
            return Err(TrapCode::UnreachableCodeReached);
        }

        result
    }

    pub fn resume(&mut self, exit_code: i32) -> Result<(), TrapCode> {
        let Some(mut outcome) = self.state.take() else {
            unreachable!("missing interrupted state, interruption should never happen inside system contracts");
        };
        let mut store_mut = self
            .compiled_runtime
            .as_mut()
            .unwrap()
            .store
            .as_context_mut();

        // Here we need to remap interruption result into the custom struct because we need to
        // pass information about fuel consumed and exit code into the runtime.
        // That is why we move return data into the output and serialize output into the return data.
        let data_mut = store_mut.data_mut();
        outcome.output = take(&mut data_mut.execution_result.return_data).into();
        outcome.exit_code = exit_code;
        let outcome = bincode::encode_to_vec(&outcome, bincode::config::legacy()).unwrap();
        data_mut.execution_result.return_data = outcome;

        // Make sure the runtime is always clear before resuming the call, because output is used
        // to pass interruption params in case of interruption
        data_mut.clear_output();

        // Since we don't suppose native interruptions inside system runtimes then we just re-call
        // execute, but with passed return data with interruption outcome.
        //
        // Possible scenarios:
        // 1. w/ return data - new frame call
        // 2. w/o return data - current frame interruption outcome
        self.execute()
    }

    pub fn try_consume_fuel(&mut self, fuel: u64) -> Result<(), TrapCode> {
        self.state.as_mut().unwrap().fuel_consumed += fuel;
        Ok(())
    }

    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        let compiled_runtime = self.compiled_runtime.as_mut().unwrap();
        compiled_runtime
            .memory
            .write(compiled_runtime.store.as_context_mut(), offset, data)
            .map_err(|_| TrapCode::MemoryOutOfBounds)
    }

    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        let compiled_runtime = self.compiled_runtime.as_ref().unwrap();
        compiled_runtime
            .memory
            .read(&compiled_runtime.store, offset, buffer)
            .map_err(|_| TrapCode::MemoryOutOfBounds)
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        // We don't support fuel for this runtime
        None
    }

    pub fn with_compiled_runtime_mut<R, F: FnOnce(&mut CompiledRuntime) -> R>(
        &mut self,
        func: F,
    ) -> R {
        let compiled_runtime = self.compiled_runtime.as_mut().unwrap();
        func(compiled_runtime)
    }

    pub fn compiled_runtime_mut(&mut self) -> &mut CompiledRuntime {
        self.compiled_runtime.as_mut().unwrap()
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
        unimplemented!()
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
