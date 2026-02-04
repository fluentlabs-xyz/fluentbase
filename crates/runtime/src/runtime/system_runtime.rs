//! System runtime backed by Wasmtime.
//!
//! This module implements **system runtimes** (trusted, privileged rWasm programs) executed via
//! Wasmtime. The key difference from `ContractRuntime` is that system runtimes:
//! - may be reused across multiple calls (store/instance caching),
//! - signal "soft exits" via *returned output* rather than trapping/unwinding.
//!
//! ## Fuel metering modes
//!
//! System runtimes support two fuel metering strategies:
//!
//! 1. **Self-metering** (`consume_fuel=false`): The contract manages fuel internally by calling
//!    the `_charge_fuel` syscall. Wasmtime fuel metering is disabled. This is used by runtimes
//!    like EVM_RUNTIME and SVM_RUNTIME that have their own gas accounting.
//!
//! 2. **Engine-metered** (`consume_fuel=true`): Wasmtime automatically meters fuel for both
//!    wasm instructions and builtin syscalls. This is used by precompiles that don't self-meter:
//!    NITRO_VERIFIER, OAUTH2_VERIFIER, WASM_RUNTIME, WEBAUTHN_VERIFIER.
//!
//! ## Why "return output" instead of trapping?
//! Some system runtimes intentionally avoid `trap` / `halt` paths because they do not always unwind
//! the stack in the desired way for this environment. Instead, they return an output buffer where
//! the first 4 bytes are the little-endian `exit_code`, and the remaining bytes are the payload.
//!
//! ## Caching model
//! - `COMPILED_MODULES` caches compiled `wasmtime::Module` by code hash globally.
//! - `COMPILED_RUNTIMES` caches instantiated `CompiledRuntime` per thread (thread-local) by code hash.
//!
//! The store is reused, so the `RuntimeContext` is swapped in/out on every call.

use crate::{syscall_handler::invoke_runtime_handler, RuntimeContext};
use fluentbase_types::{ExitCode, HashMap, SysFuncIdx, B256, STATE_DEPLOY, STATE_MAIN};
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

/// A system runtime instance.
///
/// System runtimes are **trusted** programs executed through Wasmtime and reused across calls.
/// This wrapper owns:
/// - a cached, compiled+instantiated Wasmtime runtime (`CompiledRuntime`),
/// - the per-call `RuntimeContext` (`ctx`) which is swapped into the cached store on execution,
/// - an optional resumable interruption state used when system runtimes request an interruption.
/// - a flag indicating whether Wasmtime fuel metering is enabled (`consume_fuel`).
///
/// The runtime is keyed by `code_hash` so that we can cache compiled artifacts and instances.
pub struct SystemRuntime {
    /// Cached compiled runtime (Module + Store + Instance + Memory + entry functions).
    ///
    /// NOTE: This is currently not `no_std` friendly due to Wasmtime usage.
    /// The intent is to replace/relax this once rWasm ships an optimized embedded backend.
    ///
    /// TODO(dmitry123): Compiled runtime breaks `no_std` compliance. Fix once we have an optimized
    ///  wasmtime integration inside the rWasm repository (or an alternative backend).
    compiled_runtime: Rc<RefCell<CompiledRuntime>>,

    /// Per-call execution context.
    ///
    /// This context is swapped into the cached store before execution and swapped back after,
    /// so that a single cached store/instance can serve multiple contract calls sequentially.
    ctx: RuntimeContext,

    /// Code hash of the system runtime program.
    ///
    /// Used as a cache key for both compiled modules and instantiated runtimes.
    code_hash: B256,

    /// Whether Wasmtime fuel metering is enabled for this runtime.
    ///
    /// When `true`, the engine automatically charges fuel for wasm instructions and syscalls.
    /// When `false`, the contract is expected to self-meter via `_charge_fuel` syscall.
    consume_fuel: bool,
}

/// Fully initialized compiled runtime artifacts.
///
/// This structure is cached and reused. It contains:
/// - a compiled Wasmtime `Module`,
/// - a `Store<RuntimeContext>` holding runtime state and a swap-in context,
/// - an instantiated `Instance` and its exported memory,
/// - cached exported entry functions.
struct CompiledRuntime {
    module: Module,
    store: Store<RuntimeContext>,
    instance: Instance,
    memory: Memory,
    deploy_func: Func,
    main_func: Func,
}

thread_local! {
    /// Thread-local cache of fully instantiated runtimes keyed by code hash.
    ///
    /// We keep this thread-local because Wasmtime components are not generally cheap to share across
    /// threads without careful synchronization, and because per-thread reuse is often sufficient.
    pub static COMPILED_RUNTIMES: RefCell<HashMap<B256, Rc<RefCell<CompiledRuntime>>>> =
        RefCell::new(HashMap::new());
}

impl SystemRuntime {
    /// Returns a compiled Wasmtime module for the given rWasm module (by code hash).
    ///
    /// Uses a global cache (`COMPILED_MODULES`) to avoid recompilation costs.
    /// Double-checked locking is used to minimize write lock contention.
    ///
    /// ## Fuel metering
    ///
    /// When `consume_fuel=true`, the module is compiled using `compile_wasmtime_module` with
    /// `consume_fuel=true` and `builtins_consume_fuel=true`. This enables automatic fuel
    /// charging for both wasm instructions and syscall invocations.
    ///
    /// When `consume_fuel=false`, the module is compiled with the standard engine where
    /// fuel metering is disabled. The contract is expected to self-meter.
    pub fn compiled_module(
        code_hash: B256,
        rwasm_module: RwasmModule,
        consume_fuel: bool,
    ) -> Module {
        static COMPILED_MODULES: OnceLock<RwLock<HashMap<B256, Module>>> = OnceLock::new();
        let compiled_modules = COMPILED_MODULES.get_or_init(|| RwLock::new(HashMap::new()));

        // Fast path: read lock lookup.
        {
            let guard = compiled_modules.read().unwrap();
            if let Some(module) = guard.get(&code_hash) {
                return module.clone();
            }
        }

        // Slow path: compile and insert under write lock (with re-check).
        let mut guard = compiled_modules.write().unwrap();
        if let Some(module) = guard.get(&code_hash) {
            return module.clone();
        }

        // `hint_section` contains Wasmtime-compatible wasm bytes for the system runtime.
        // Any compilation failure here is fatal: genesis/runtime packaging is inconsistent.
        let module = if consume_fuel {
            Module::new(
                wasmtime_engine_with_consume_fuel(),
                &rwasm_module.hint_section,
            )
            .unwrap()
        } else {
            Module::new(wasmtime_engine(), &rwasm_module.hint_section).unwrap()
        };
        guard.insert(code_hash, module.clone());
        module
    }

    /// Clears the per-thread cache of instantiated runtimes.
    ///
    /// Useful in tests or when a process needs to drop cached instances (e.g. after an upgrade).
    pub fn reset_cached_runtimes() {
        COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
            compiled_runtimes.clear();
        });
    }

    /// Creates a new `SystemRuntime`.
    ///
    /// If a compiled runtime for `code_hash` is present in the thread-local cache, it will be reused.
    /// Otherwise, this function compiles/loads the module and instantiates it with imports wired via
    /// `import_linker`.
    ///
    /// ## Fuel metering
    ///
    /// The `consume_fuel` parameter determines whether Wasmtime fuel metering is enabled:
    /// - `true`: Engine automatically meters fuel (for NITRO, OAUTH2, WASM_RUNTIME, WEBAUTHN)
    /// - `false`: Contract self-meters via `_charge_fuel` syscall (for EVM_RUNTIME, etc.)
    pub fn new(
        module: RwasmModule,
        import_linker: Arc<ImportLinker>,
        code_hash: B256,
        ctx: RuntimeContext,
        consume_fuel: bool,
    ) -> Self {
        let compiled_runtime = COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
            if let Some(compiled_runtime) = compiled_runtimes.get(&code_hash).cloned() {
                return compiled_runtime;
            }

            let module = Self::compiled_module(code_hash, module, consume_fuel);

            // IMPORTANT: Engine must be obtained FROM the module to ensure store uses
            // the same engine with correct consume_fuel configuration.
            let engine = module.engine();
            let linker = wasmtime_import_linker(engine, import_linker);

            // NOTE: store starts with a default context and receives the actual per-call context
            // via `swap` inside `execute`.
            let mut store = Store::new(engine, RuntimeContext::default());

            let instance = linker.instantiate(store.as_context_mut(), &module).unwrap();

            // System runtimes must expose both entrypoints.
            let deploy_func = instance.get_func(store.as_context_mut(), "deploy").unwrap();
            let main_func = instance.get_func(store.as_context_mut(), "main").unwrap();

            // System runtimes must export linear memory under the name `memory`.
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
            consume_fuel,
        }
    }

    /// Executes the system runtime entrypoint and updates `self.ctx.execution_result`.
    ///
    /// Execution uses the cached store/instance. Before calling into Wasmtime, we swap
    /// `self.ctx` into the store to ensure syscalls and state access refer to the correct context.
    ///
    /// ## Fuel metering
    ///
    /// If `consume_fuel=true`, the fuel limit is set in the store before execution. Wasmtime
    /// will automatically decrement fuel as instructions execute.
    ///
    /// ## Error handling model
    /// - If Wasmtime traps unexpectedly, we **do not propagate** the trap outward as fatal.
    ///   Instead, we mark `UnexpectedFatalExecutionFailure` in `execution_result` and return `Ok(())`
    ///   so the outer executor can treat it as a partially controlled failure.
    /// - Normal completion is signaled by output where the first 4 bytes are LE `exit_code`.
    /// - Interruption is requested by returning `ExitCode::InterruptionCalled` in that header.
    pub fn execute(&mut self) -> Result<(), TrapCode> {
        let mut compiled_runtime = self.compiled_runtime.borrow_mut();

        // Rewrite runtime context before each call, since we reuse the same store/runtime instance
        // across multiple calls. We must replace whatever context was left from the previous call.
        //
        // Safety: Calls into a cached runtime must be strictly sequential. No reentrancy or
        // overlapping calls are allowed because we swap a single `RuntimeContext` in/out.
        core::mem::swap(compiled_runtime.store.data_mut(), &mut self.ctx);

        // If fuel metering is enabled, set the fuel limit before execution.
        // The store is reused, so we must reset fuel for each new call.
        if self.consume_fuel {
            let fuel_limit = compiled_runtime.store.data().fuel_limit;
            compiled_runtime
                .store
                .set_fuel(fuel_limit)
                .expect("failed to set fuel limit");
        }

        // Choose entrypoint based on the current execution state.
        let entrypoint = match compiled_runtime.store.data().state {
            STATE_MAIN => compiled_runtime.main_func,
            STATE_DEPLOY => compiled_runtime.deploy_func,
            _ => unreachable!(),
        };

        let result = entrypoint
            .call(compiled_runtime.store.as_context_mut(), &[], &mut [])
            .map_err(map_anyhow_error);

        // Always swap back immediately after the call so we keep `self.ctx` authoritative.
        core::mem::swap(compiled_runtime.store.data_mut(), &mut self.ctx);

        // If Wasmtime trapped, treat it as an unexpected fatal failure and degrade into a safe
        // error code. This avoids propagating a raw trap across the execution boundary.
        if let Err(trap_code) = result.as_ref() {
            // OutOfFuel is expected for engine-metered precompiles when fuel is exhausted.
            if *trap_code == TrapCode::OutOfFuel {
                return Err(TrapCode::OutOfFuel);
            }

            let exit_code = ExitCode::from(self.ctx.execution_result.exit_code);

            if exit_code == ExitCode::Panic {
                eprintln!(
                    "runtime: system execution panicked: {} (investigate)",
                    core::str::from_utf8(&self.ctx.execution_result.output)
                        .unwrap_or("unable to decode UTF-8 panic message")
                );
            } else if exit_code != ExitCode::Ok {
                eprintln!(
                    "runtime: system execution failed with exit code: {} ({}) (investigate)",
                    exit_code, self.ctx.execution_result.exit_code
                );
            }

            eprintln!(
                "runtime: unexpected trap inside system runtime: {:?} ({}) (investigate)",
                trap_code, trap_code,
            );

            self.ctx.execution_result.exit_code =
                ExitCode::UnexpectedFatalExecutionFailure.into_i32();
            return Ok(());
        }

        // System runtimes return output prefixed with an LE i32 exit code.
        //
        // Note: System runtimes avoid trapping for control flow because trapping/halt may not unwind
        // the stack as required in this environment.
        let output = take(&mut self.ctx.execution_result.output);
        if output.len() < 4 {
            eprintln!(
                "runtime: unexpected output size from system runtime: {} (investigate)",
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

        // If exit code indicates interruption, convert it into a trap that the outer executor
        // understands (`TrapCode::InterruptionCalled`).
        //
        // Safety: `InterruptionCalled` is expected only from trusted system runtimes.
        // Untrusted contracts might use the same numeric code but will not be executed here.
        if ExitCode::from_repr(exit_code) == Some(ExitCode::InterruptionCalled) {
            // Move output into return_data. For system runtimes we don't expose a dedicated
            // "interrupt params" ABI, so we treat the returned output payload as the interruption
            // parameters.
            self.ctx.execution_result.return_data = take(&mut self.ctx.execution_result.output);

            assert!(
                !self.ctx.execution_result.return_data.is_empty(),
                "runtime: interruption payload must not be empty"
            );

            return Err(TrapCode::InterruptionCalled);
        }

        result
    }

    /// Resumes execution after an interruption.
    ///
    /// System runtimes do not support "native" resumable interruptions internally in the same way
    /// as contract runtimes. Therefore, resume currently re-enters `execute()` after clearing any
    /// stale output.
    ///
    /// NOTE: `exit_code` and `fuel_consumed` are intentionally ignored here because fuel metering
    /// is handled by `RuntimeContext`, and exit codes are encoded in returned output.
    pub fn resume(&mut self, _exit_code: i32, _fuel_consumed: u64) -> Result<(), TrapCode> {
        // Ensure the output is clear before resuming; output is used to carry interruption params.
        self.ctx.clear_output();

        // Re-enter execution. Possible scenarios:
        // 1) With return_data: new frame call.
        // 2) Without return_data: current frame interruption outcome.
        self.execute()
    }

    /// Writes bytes into the system runtime's linear memory.
    ///
    /// Bounds violations are mapped into `TrapCode::MemoryOutOfBounds`.
    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        let mut compiled_runtime = self.compiled_runtime.borrow_mut();
        let memory = compiled_runtime.memory;
        memory
            .write(&mut compiled_runtime.store, offset, data)
            .map_err(|_| TrapCode::MemoryOutOfBounds)
    }

    /// Reads bytes from the system runtime's linear memory.
    ///
    /// Bounds violations are mapped into `TrapCode::MemoryOutOfBounds`.
    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        let compiled_runtime = self.compiled_runtime.borrow();
        compiled_runtime
            .memory
            .read(&compiled_runtime.store, offset, buffer)
            .map_err(|_| TrapCode::MemoryOutOfBounds)
    }

    /// Returns remaining fuel, if fuel metering is enabled.
    ///
    /// For engine-metered precompiles (`consume_fuel=true`), returns the actual remaining fuel
    /// from the Wasmtime store.
    ///
    /// For self-metering runtimes (`consume_fuel=false`), returns `None` because fuel is
    /// tracked in `RuntimeContext` via `_charge_fuel` syscall, not in the Wasmtime store.
    pub fn remaining_fuel(&self) -> Option<u64> {
        if self.consume_fuel {
            let compiled_runtime = self.compiled_runtime.borrow();
            let fuel = compiled_runtime.store.get_fuel().ok();
            eprintln!(
                "DEBUG: remaining_fuel={:?}, fuel_limit={}",
                fuel,
                compiled_runtime.store.data().fuel_limit
            );
            fuel
        } else {
            None
        }
    }

    /// Provides mutable access to the per-call runtime context.
    pub fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        func(&mut self.ctx)
    }

    /// Provides immutable access to the per-call runtime context.
    pub fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        func(&self.ctx)
    }
}

/// Returns the shared Wasmtime engine instance.
///
/// The engine is configured once and reused globally.
/// Fuel metering is disabled (`consume_fuel(false)`) because fuel is accounted
/// inside `RuntimeContext` and system runtimes are expected to self-manage.
fn wasmtime_engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut cfg = Config::new();
        cfg.strategy(Strategy::Cranelift);
        cfg.collector(wasmtime::Collector::Null);

        // rWasm stack size is defined in 32-bit slots; Wasmtime expects bytes.
        cfg.max_wasm_stack(N_MAX_STACK_SIZE * size_of::<u32>());

        cfg.async_support(false);
        cfg.wasm_memory64(false);
        cfg.memory_init_cow(false);
        cfg.cranelift_opt_level(OptLevel::Speed);
        cfg.parallel_compilation(true);

        // Fuel accounting is handled externally via RuntimeContext.
        cfg.consume_fuel(false);

        Engine::new(&cfg).unwrap()
    })
}

/// Builds wasmtime syscall fuel params from ImportLinker.
///
/// TODO(d1r1): move to rwasm crate as a method on ImportLinker
fn build_syscall_fuel_params(
    import_linker: &ImportLinker,
) -> std::collections::HashMap<wasmtime::SyscallName, wasmtime::SyscallFuelParams> {
    use wasmtime::{LinearFuelParams, QuadraticFuelParams, SyscallFuelParams, SyscallName};

    let mut params = std::collections::HashMap::new();
    for (import_name, import_entity) in import_linker.iter() {
        let syscall_name = SyscallName {
            module: import_name.module().to_string(),
            name: import_name.name().to_string(),
        };

        let fuel_param = match &import_entity.syscall_fuel_param {
            rwasm::SyscallFuelParams::None => continue,
            rwasm::SyscallFuelParams::Const(base) => SyscallFuelParams::Const(*base),
            rwasm::SyscallFuelParams::LinearFuel(p) => {
                SyscallFuelParams::LinearFuel(LinearFuelParams {
                    base_fuel: p.base_fuel,
                    word_cost: p.word_cost,
                    linear_param_index: p.param_index,
                    max_linear: p.max_linear,
                })
            }
            rwasm::SyscallFuelParams::QuadraticFuel(p) => {
                SyscallFuelParams::QuadraticFuel(QuadraticFuelParams {
                    local_depth: p.param_index,
                    word_cost: p.word_cost,
                    divisor: p.divisor,
                    max_quadratic: p.max_quadratic,
                    fuel_denom_rate: p.fuel_denom_rate,
                })
            }
        };

        params.insert(syscall_name, fuel_param);
    }
    params
}

/// Returns the shared Wasmtime engine instance with fuel metering enabled.
///
/// Used for engine-metered precompiles (NITRO, OAUTH2, WASM_RUNTIME, WEBAUTHN).
/// These precompiles don't self-meter via `_charge_fuel` syscall, so the engine
/// must automatically charge fuel for:
/// - wasm instructions (via `consume_fuel=true`)
/// - syscall/builtin calls (via `syscall_fuel_params`)
///
/// TODO(d1r1): move to rwasm crate - add async_support parameter to
/// factory_wasmtime_engine_with_linker and reuse it here
fn wasmtime_engine_with_consume_fuel() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let import_linker = fluentbase_types::import_linker_v1_preview();

        let mut cfg = Config::new();
        cfg.strategy(Strategy::Cranelift);
        cfg.collector(wasmtime::Collector::Null);
        cfg.max_wasm_stack(N_MAX_STACK_SIZE * size_of::<u32>());
        cfg.async_support(false); // Must be false for sync instantiate()/call()
        cfg.wasm_memory64(false);
        cfg.memory_init_cow(false);
        cfg.cranelift_opt_level(OptLevel::Speed);
        cfg.parallel_compilation(true);
        cfg.consume_fuel(true);
        cfg.syscall_fuel_params(build_syscall_fuel_params(&import_linker));

        Engine::new(&cfg).unwrap()
    })
}

/// Adapter that exposes the `rwasm::Store` interface over a Wasmtime `Caller`.
///
/// This is used by the Wasmtime import trampoline to provide system calls with
/// access to memory and runtime context.
struct CallerAdapter<'a> {
    caller: wasmtime::Caller<'a, RuntimeContext>,
}

impl<'a> rwasm::Store<RuntimeContext> for CallerAdapter<'a> {
    /// Reads bytes from exported linear memory.
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

    /// Writes bytes into exported linear memory.
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

    /// Provides mutable access to runtime context stored inside the Wasmtime store.
    fn context_mut<R, F: FnOnce(&mut RuntimeContext) -> R>(&mut self, func: F) -> R {
        func(self.caller.data_mut())
    }

    /// Provides immutable access to runtime context stored inside the Wasmtime store.
    fn context<R, F: FnOnce(&RuntimeContext) -> R>(&self, func: F) -> R {
        func(self.caller.data())
    }

    /// Charges fuel from this store, if applicable.
    ///
    /// For self-metering runtimes, fuel is handled via RuntimeContext.
    /// For engine-metered runtimes, fuel is automatically charged by Wasmtime.
    /// In both cases, this callback is a no-op.
    fn try_consume_fuel(&mut self, _delta: u64) -> Result<(), TrapCode> {
        Ok(())
    }

    /// Returns remaining fuel computed from context accounting.
    fn remaining_fuel(&self) -> Option<u64> {
        let ctx = self.caller.data();
        Some(ctx.fuel_limit - ctx.execution_result.fuel_consumed)
    }
}

/// Creates a Wasmtime linker from an rWasm `ImportLinker`.
///
/// Each imported function becomes a Wasmtime host function that:
/// - maps Wasmtime values to rWasm values,
/// - invokes `invoke_runtime_handler`,
/// - maps rWasm results back to Wasmtime values,
/// - converts certain trap codes into controlled termination (`ExecutionHalted`).
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

/// Wasmtime import trampoline that executes a single runtime syscall.
///
/// Maps input params and results between Wasmtime (`Val`) and rWasm (`Value`),
/// then calls `invoke_runtime_handler` with a `CallerAdapter` providing memory/context access.
///
/// Returns `Ok(())` on success, or an `anyhow::Error` that may wrap a trap.
fn wasmtime_syscall_handler<'a>(
    sys_func_idx: u32,
    caller: wasmtime::Caller<'a, RuntimeContext>,
    params: &[Val],
    result: &mut [Val],
) -> anyhow::Result<()> {
    // Convert input values from Wasmtime format into rWasm format.
    let mut buffer = SmallVec::<[Value; 32]>::new();
    buffer.extend(params.iter().map(|x| match x {
        Val::I32(value) => Value::I32(*value),
        Val::I64(value) => Value::I64(*value),
        Val::F32(value) => Value::F32(F32::from_bits(*value)),
        Val::F64(value) => Value::F64(F64::from_bits(*value)),
        _ => unreachable!("wasmtime: unsupported type: {:?}", x),
    }));

    // Reserve space for result values (initialized to zeros).
    buffer.extend(core::iter::repeat(Value::I32(0)).take(result.len()));

    let (mapped_params, mapped_result) = buffer.split_at_mut(params.len());

    // Caller adapter provides memory/context operations expected by `invoke_runtime_handler`.
    let mut caller_adapter = CallerAdapter::<'a> { caller };

    let sys_func_idx =
        SysFuncIdx::from_repr(sys_func_idx).ok_or(TrapCode::UnknownExternalFunction)?;

    let syscall_result = invoke_runtime_handler(
        &mut caller_adapter,
        sys_func_idx,
        mapped_params,
        mapped_result,
    );

    // Treat `ExecutionHalted` as a controlled termination rather than a hard error.
    let should_terminate = syscall_result.map(|_| false).or_else(|trap_code| {
        if trap_code == TrapCode::ExecutionHalted {
            Ok(true)
        } else {
            Err(trap_code)
        }
    })?;

    // Map all values back to Wasmtime format.
    for (i, value) in mapped_result.iter().enumerate() {
        result[i] = match value {
            Value::I32(value) => Val::I32(*value),
            Value::I64(value) => Val::I64(*value),
            Value::F32(value) => Val::F32(value.to_bits()),
            Value::F64(value) => Val::F64(value.to_bits()),
            _ => unreachable!("wasmtime: unsupported type: {:?}", value),
        };
    }

    // Terminate execution if requested.
    if should_terminate {
        return Err(TrapCode::ExecutionHalted.into());
    }

    Ok(())
}

/// Maps `anyhow::Error` coming from Wasmtime into an rWasm `TrapCode`.
///
/// - If the error is a Wasmtime `Trap`, it is mapped into the closest `TrapCode`.
/// - If the error already contains a `TrapCode`, it is returned as-is.
/// - Otherwise the error is treated as an illegal opcode (fallback).
fn map_anyhow_error(err: anyhow::Error) -> TrapCode {
    if let Some(trap) = err.downcast_ref::<Trap>() {
        eprintln!("wasmtime trap: {:?}", trap);

        // Map Wasmtime trap codes into rWasm trap codes.
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
            Trap::AlwaysTrapAdapter => unreachable!("component model is not supported"),
            Trap::OutOfFuel => TrapCode::OutOfFuel,
            Trap::AtomicWaitNonSharedMemory => unreachable!("atomics are not supported"),
            Trap::NullReference => TrapCode::IndirectCallToNull,
            Trap::ArrayOutOfBounds | Trap::AllocationTooLarge => {
                unreachable!("GC is not supported")
            }
            Trap::CastFailure => TrapCode::BadConversionToInteger,
            Trap::CannotEnterComponent => unreachable!("component model is not supported"),
            Trap::NoAsyncResult => unreachable!("async mode must be disabled"),
            _ => unreachable!("unknown Wasmtime trap"),
        }
    } else if let Some(trap) = err.downcast_ref::<TrapCode>() {
        // Our own trap code was propagated through anyhow; pass it through.
        *trap
    } else {
        eprintln!("wasmtime: unknown error: {:?}", err);

        // TODO(dmitry123): Decide which trap code is the best fallback for unknown Wasmtime errors.
        TrapCode::IllegalOpcode
    }
}

/// Maps an rWasm `ValType` into a Wasmtime `ValType`.
///
/// System runtimes currently support only numeric scalar types.
fn map_val_type(val_type: ValType) -> wasmtime::ValType {
    match val_type {
        ValType::I32 => wasmtime::ValType::I32,
        ValType::I64 => wasmtime::ValType::I64,
        ValType::F32 => wasmtime::ValType::F32,
        ValType::F64 => wasmtime::ValType::F64,
        _ => unreachable!("wasmtime: unsupported type: {:?}", val_type),
    }
}
