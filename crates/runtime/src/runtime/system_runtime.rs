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

use crate::{syscall_handler::runtime_syscall_handler, RuntimeContext};
use alloc::sync::Arc;
use core::{cell::RefCell, mem::take};
use fluentbase_types::{ExitCode, HashMap, SysFuncIdx, B256, STATE_DEPLOY, STATE_MAIN};
use rwasm::{
    CompilationConfig, ImportLinker, Opcode, RwasmModule, StateRouterConfig, StoreTr,
    StrategyDefinition, StrategyExecutor, TrapCode, Value, N_MAX_ALLOWED_MEMORY_PAGES,
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
    compiled_runtime: Arc<RefCell<CompiledRuntime>>,

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
type CompiledRuntime = StrategyExecutor<RuntimeContext>;

thread_local! {
    /// Thread-local cache of fully instantiated runtimes keyed by code hash.
    ///
    /// We keep this thread-local because Wasmtime components are not generally inexpensive to share across
    /// threads without careful synchronization, and because per-thread reuse is often enough.
    pub static COMPILED_RUNTIMES: RefCell<HashMap<B256, Arc<RefCell<CompiledRuntime>>>> =
        RefCell::new(HashMap::new());
}

impl SystemRuntime {
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
        rwasm_module: RwasmModule,
        import_linker: Arc<ImportLinker>,
        code_hash: B256,
        ctx: RuntimeContext,
        consume_fuel: bool,
    ) -> Self {
        let compiled_runtime = COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
            if let Some(compiled_runtime) = compiled_runtimes.get(&code_hash).cloned() {
                return compiled_runtime;
            }

            let config = CompilationConfig::default()
                .with_state_router(StateRouterConfig {
                    states: Box::new([
                        ("deploy".into(), STATE_DEPLOY),
                        ("main".into(), STATE_MAIN),
                    ]),
                    opcode: Some(Opcode::Call(SysFuncIdx::STATE as u32)),
                })
                .with_import_linker(import_linker.clone())
                .with_allow_malformed_entrypoint_func_type(true)
                .with_consume_fuel(consume_fuel)
                .with_builtins_consume_fuel(false)
                .with_max_allowed_memory_pages(N_MAX_ALLOWED_MEMORY_PAGES);
            // `hint_section` contains Wasmtime-compatible wasm bytes for the system runtime.
            // Any compilation failure here is fatal: genesis/runtime packaging is inconsistent.
            let typed_module =
                StrategyDefinition::new(config, &rwasm_module.hint_section, Some(code_hash.0))
                    .expect("runtime: failed to compile system runtime module");
            let Ok(executor) = typed_module.create_executor(
                import_linker,
                RuntimeContext::default(),
                runtime_syscall_handler,
                // We can't set a fuel limit here because it's not known until execution.
                None,
                Some(N_MAX_ALLOWED_MEMORY_PAGES),
            ) else {
                unreachable!("runtime: failed to create executor for system runtime module")
            };

            #[allow(clippy::arc_with_non_send_sync)]
            let compiled_runtime = Arc::new(RefCell::new(executor));
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
        core::mem::swap(compiled_runtime.data_mut(), &mut self.ctx);

        // If fuel metering is enabled, set the fuel limit before execution.
        // The store is reused, so we must reset fuel for each new call.
        let fuel_limit = compiled_runtime.data().fuel_limit;
        compiled_runtime.reset_fuel(fuel_limit);

        // Choose an entrypoint based on the current execution state.
        let entrypoint = match compiled_runtime.data().state {
            STATE_MAIN => "main",
            STATE_DEPLOY => "deploy",
            _ => unreachable!(),
        };

        let mut output = [Value::I32(0)];
        // Rust generates a C-style `main(argc: i32, argv: i32) -> i32` signature for wasm targets when `main` returns `i32`.
        // Even in `no_std`, the toolchain follows the traditional C/WASI ABI convention where `argc` and `argv` are passed
        // as 32-bit values (wasm pointers).
        //
        // We do not use command-line arguments in this environment, so we pass `0, 0` as dummy values.
        // The generated shim ignores them unless argument handling is explicitly implemented.
        //
        // https://reviews.llvm.org/D70700
        let result = if entrypoint == "main" {
            compiled_runtime.execute(entrypoint, &[Value::I32(0), Value::I32(0)], &mut output)
        } else {
            compiled_runtime.execute(entrypoint, &[], &mut output)
        };
        let exit_code = output[0].i32().unwrap();

        // Always swap back immediately after the call, so we keep `self.ctx` authoritative.
        core::mem::swap(compiled_runtime.data_mut(), &mut self.ctx);

        // The application can return trap code though exit code, we should handle such cases as well
        if self.ctx.execution_result.exit_code != ExitCode::Ok.into_i32() {
            // If panic happens, then we can only forward into output
            if self.ctx.execution_result.exit_code == ExitCode::Panic.into_i32() {
                eprintln!(
                    "WARN: system execution panicked: {} (investigate)",
                    core::str::from_utf8(&self.ctx.execution_result.output)
                        .unwrap_or("unable to decode UTF-8 panic message")
                );
            }
            // We assume any not `Ok` error can happen, for example, due to OOM (because our EVM runtime is limited with 64mB only),
            // but we should handle it gracefully if it happens. When it happens, we have a corrupted state: stack and memory.
            // Ideally, we should terminate the latest frame and expect the caller to continue its execution because we can free
            // resources we consumed. But here we just terminate the runtime entirely. It means that all previous calls
            // will fail because they lose their state. It's the best here because we automatically terminate all nested
            // execution to avoid potential memory or stack access violations.
            COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
                compiled_runtimes.remove(&self.code_hash);
            });
            // We return `Ok` here because the exit code is already set
            return Ok(());
        } else {
            self.ctx.execution_result.exit_code = exit_code;
        }

        // If wasmtime trapped, treat it as an unexpected fatal failure and degrade into a safe
        // error code. This avoids propagating a raw trap across the execution boundary.
        if let Err(trap_code) = result.as_ref() {
            // The trap `OutOfFuel` is expected for engine-metered precompiles when fuel is exhausted.
            if *trap_code == TrapCode::OutOfFuel {
                // This case is tricky, because if it happens, then we might have corrupted stack and
                // uncleaned memory. Since we can't handle it gracefully, then we can only reset the existing
                // runtime to make sure memory and stack are clean. There is no ddos attack here because,
                // to achieve this, the user must pay a penalty.
                COMPILED_RUNTIMES.with_borrow_mut(|compiled_runtimes| {
                    compiled_runtimes.remove(&self.code_hash);
                });
                // Forward the `OutOfFuel` trap to the outer executor, so it can handle it gracefully.
                return Err(*trap_code);
            }
            eprintln!(
                "runtime: unexpected trap inside system runtime: {:?} ({}) (investigate)",
                trap_code, trap_code,
            );
            self.ctx.execution_result.exit_code =
                ExitCode::UnexpectedFatalExecutionFailure.into_i32();
            return Ok(());
        }

        // If exit code indicates an interruption, convert it into a trap that the outer executor
        // understands (`TrapCode::InterruptionCalled`).
        //
        // Safety: `InterruptionCalled` is expected only from trusted system runtimes.
        // Untrusted contracts might use the same numeric code but will not be executed here.
        if exit_code == ExitCode::InterruptionCalled.into_i32() {
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
    /// Note: `exit_code` and `fuel_consumed` are intentionally ignored here because fuel metering
    /// is handled by `RuntimeContext`, and exit codes are encoded in returned output.
    pub fn resume(&mut self, _exit_code: i32, _fuel_consumed: u64) -> Result<(), TrapCode> {
        // Ensure the output is clear before resuming; output is used to carry interruption params.
        self.ctx.clear_output();

        // Re-enter execution. Possible scenarios:
        // 1) With return_data: current frame interruption outcome.
        // 2) Without return_data: new frame call.
        self.execute()
    }

    /// Writes bytes into the system runtime's linear memory.
    ///
    /// Bounds violations are mapped into `TrapCode::MemoryOutOfBounds`.
    pub fn memory_write(&mut self, offset: usize, data: &[u8]) -> Result<(), TrapCode> {
        let mut compiled_runtime = self.compiled_runtime.borrow_mut();
        compiled_runtime.memory_write(offset, data)
    }

    /// Reads bytes from the system runtime's linear memory.
    ///
    /// Bounds violations are mapped into `TrapCode::MemoryOutOfBounds`.
    pub fn memory_read(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), TrapCode> {
        let mut compiled_runtime = self.compiled_runtime.borrow_mut();
        compiled_runtime.memory_read(offset, buffer)
    }

    /// Returns remaining fuel if fuel metering is enabled.
    ///
    /// For engine-metered precompiles (`consume_fuel=true`), returns the actual remaining fuel
    /// from the Wasmtime store.
    ///
    /// For self-metering runtimes (`consume_fuel=false`), returns `None` because fuel is
    /// tracked in `RuntimeContext` via `_charge_fuel` syscall, not in the Wasmtime store.
    pub fn remaining_fuel(&self) -> Option<u64> {
        let compiled_runtime = self.compiled_runtime.borrow();
        if self.consume_fuel {
            compiled_runtime.remaining_fuel()
        } else {
            None
        }
    }

    /// Provides mutable access to the per-call runtime context.
    pub fn context_mut(&mut self) -> &mut RuntimeContext {
        &mut self.ctx
    }

    /// Provides immutable access to the per-call runtime context.
    pub fn context(&self) -> &RuntimeContext {
        &self.ctx
    }
}
