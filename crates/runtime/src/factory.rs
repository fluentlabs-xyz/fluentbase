use crate::{ExecutionResult, Runtime, RuntimeResult};
use fluentbase_types::{import_linker_v1_preview, BytecodeOrHash, HashMap, B256};
use rwasm::{ExecutionEngine, ImportLinker, RwasmModule, Strategy};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

/// Global factory maintaining compiled module cache and resumable runtime instances.
pub struct RuntimeFactory {
    /// Cache of compiled modules keyed by code hash.
    /// TODO(dmitry123): Add LRU cache to this map to avoid memory leak
    pub cached_modules: HashMap<B256, Arc<Strategy>>,
    /// Suspended runtimes keyed by per-transaction call identifier.
    pub recoverable_runtimes: HashMap<u32, Runtime>,
    /// An import linker
    pub import_linker: Arc<ImportLinker>,
    /// Monotonically increasing counter for assigning call identifiers.
    pub transaction_call_id_counter: AtomicU32,
}

impl RuntimeFactory {
    /// Creates a factory configured for the v1 preview import surface.
    pub fn new_v1_preview() -> Self {
        Self {
            cached_modules: HashMap::new(),
            recoverable_runtimes: HashMap::new(),
            import_linker: import_linker_v1_preview(),
            transaction_call_id_counter: AtomicU32::new(1),
        }
    }

    /// Returns a cached module for the given bytecode or compiles and caches it on first use.
    #[tracing::instrument(level = "info", skip_all, fields(bytecode_or_hash = %bytecode_or_hash))]
    pub fn get_module_or_init(&mut self, bytecode_or_hash: BytecodeOrHash) -> Arc<Strategy> {
        let code_hash = bytecode_or_hash.code_hash();
        if let Some(entry) = self.cached_modules.get(&code_hash) {
            return entry.clone();
        }

        let (address, rwasm_module) = match bytecode_or_hash {
            BytecodeOrHash::Bytecode {
                address,
                bytecode: rwasm_module,
                ..
            } => (address, rwasm_module),
            BytecodeOrHash::Hash(_hash) => {
                // TODO(dmitry123): Do we want to have lock here until resources are warmed up?
                panic!("runtime: can't compile just by hash")
            }
        };

        println!("missing strategy: code_hash={code_hash} address={address}");

        let _span = tracing::info_span!("parse_rwasm_module").entered();
        let (rwasm_module, _) = RwasmModule::new_or_empty(rwasm_module.as_ref());
        drop(_span);

        // #[cfg(feature = "wasmtime")]
        // if fluentbase_types::is_system_precompile(&address)
        //     && rwasm_module.hint_type() == rwasm::HintType::WASM
        // {
        //     return self.init_wasmtime(rwasm_module, code_hash);
        // }

        self.init_rwasm(rwasm_module, code_hash)
    }

    /// Saves the current runtime instance for later resumption and returns its call identifier.
    pub fn try_remember_runtime(
        &mut self,
        runtime_result: RuntimeResult,
        runtime: Runtime,
    ) -> ExecutionResult {
        let interruption = match runtime_result {
            RuntimeResult::Result(result) => {
                // Return result (there is no need to do anything else)
                return result;
            }
            RuntimeResult::Interruption(interruption) => interruption,
        };
        // Calculate new `call_id` counter (a runtime recover identifier)
        let call_id = self
            .transaction_call_id_counter
            .fetch_add(1, Ordering::Relaxed);
        // Remember the runtime
        self.recoverable_runtimes.insert(call_id, runtime);
        ExecutionResult {
            // We return `call_id` as exit code (it's safe, because exit code can't be positive)
            exit_code: call_id as i32,
            // Forward info about consumed and refunded fuel (during the call)
            fuel_consumed: interruption.fuel_consumed,
            fuel_refunded: interruption.fuel_refunded,
            // The output we map into return data
            output: interruption.output,
            return_data: vec![],
        }
    }

    /// Fetches and removes a previously remembered runtime by its call identifier.
    pub(crate) fn recover_runtime(&mut self, call_id: u32) -> Runtime {
        self.recoverable_runtimes
            .remove(&call_id)
            .expect("runtime: can't resolve runtime by id, it should never happen")
    }

    // #[cfg(feature = "wasmtime")]
    // #[tracing::instrument(level = "info", skip_all, fields(code_hash = %code_hash))]
    // /// Initializes a Wasmtime-based strategy for the given module and inserts it into the cache.
    // fn init_wasmtime(
    //     &mut self,
    //     rwasm_module: RwasmModule,
    //     #[allow(unused)] code_hash: B256,
    // ) -> Arc<Strategy> {
    //     // The lock helps to avoid recompiling same wasmtime modules,
    //     // if you run multiple tests in parallel
    //     #[cfg(feature = "inter-process-lock")]
    //     let _guard = crate::inter_process_lock::InterProcessLock::acquire(
    //         crate::inter_process_lock::FILE_NAME_PREFIX1,
    //         code_hash.to_string(),
    //     )
    //     .unwrap();
    //
    //     let config = fluentbase_types::default_compilation_config().with_consume_fuel(false);
    //
    //     // Theoretically, it should always compile, because we compile in advance on node start and node just won't start,
    //     // if it wasmtime modules can't be compiled (all runtimes, like evm, svm, wasm, etc.)
    //     let module = rwasm::compile_wasmtime_module(config, &rwasm_module.hint_section)
    //         .expect("failed to compile wasmtime module, this should never happen");
    //
    //     let strategy = Arc::new(Strategy::Wasmtime { module });
    //     if let Some(prev) = self.cached_modules.insert(code_hash, strategy.clone()) {
    //         drop(prev);
    //     }
    //     strategy
    // }

    #[tracing::instrument(level = "info", skip_all, fields(code_hash = %code_hash))]
    fn init_rwasm(
        &mut self,
        module: RwasmModule,
        #[allow(unused)] code_hash: B256,
    ) -> Arc<Strategy> {
        let strategy = Arc::new(Strategy::Rwasm {
            module,
            engine: ExecutionEngine::acquire_shared(),
        });
        if let Some(prev) = self.cached_modules.insert(code_hash, strategy.clone()) {
            drop(prev);
        }
        strategy
    }

    pub fn reset_call_id_counter(&mut self) {
        self.transaction_call_id_counter.store(1, Ordering::Relaxed);
    }
}
