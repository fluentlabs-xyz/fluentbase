use crate::{syscall_handler::runtime_syscall_handler, Runtime, RuntimeContext};
use fluentbase_types::{import_linker_v1_preview, BytecodeOrHash, B256};
use hashbrown::{
    hash_map::{Entry, VacantEntry},
    HashMap,
};
use rwasm::{ExecutionEngine, ImportLinker, RwasmModule, Strategy, TypedStore};
use std::{cell::RefCell, collections::LinkedList, rc::Rc, sync::Arc};

// TODO(dmitry123): What's the most optimal value here? It depends on the type of application we execute,
//  what engine is used and how many nested calls are allowed. For example, in EVM we have 1024, but what if we
//  cover only most common cases? Like 10 or 30 nested calls?
const N_DEFAULT_CACHED_STORE: usize = 0;

pub struct CachedModule {
    strategy: Arc<Strategy>,
    stores: LinkedList<TypedStore<RuntimeContext>>,
}

impl CachedModule {
    pub fn new(strategy: Strategy, import_linker: Rc<ImportLinker>) -> Self {
        let mut stores = LinkedList::new();
        for _ in 0..N_DEFAULT_CACHED_STORE {
            let store = strategy.create_store(
                import_linker.clone(),
                RuntimeContext::default(),
                runtime_syscall_handler,
            );
            stores.push_back(store);
        }
        Self {
            strategy: Arc::new(strategy),
            stores,
        }
    }

    pub fn acquire_shared(&mut self) -> (Arc<Strategy>, Option<TypedStore<RuntimeContext>>) {
        (self.strategy.clone(), self.stores.pop_front())
    }

    pub fn return_store(&mut self, store: TypedStore<RuntimeContext>) {
        self.stores.push_back(store);
    }
}

pub struct RuntimeFactory {
    // TODO(dmitry123): Add LRU cache to this map to avoid memory leak (or remove HashMap?)
    pub cached_modules: HashMap<B256, CachedModule>,
    pub recoverable_runtimes: HashMap<u32, Runtime>,
    pub import_linker: Rc<ImportLinker>,
    pub transaction_call_id_counter: u32,
}

thread_local! {
    pub static CACHING_RUNTIME_FACTORY: RefCell<RuntimeFactory> = RefCell::new(RuntimeFactory::new_v1_preview());
}

impl RuntimeFactory {
    pub fn new_v1_preview() -> Self {
        Self {
            cached_modules: HashMap::new(),
            recoverable_runtimes: HashMap::new(),
            import_linker: import_linker_v1_preview(),
            transaction_call_id_counter: 1,
        }
    }

    #[tracing::instrument(level = "info", skip_all, fields(bytecode_or_hash = %bytecode_or_hash))]
    pub fn get_module_or_init(&mut self, bytecode_or_hash: BytecodeOrHash) -> &mut CachedModule {
        let code_hash = bytecode_or_hash.code_hash();
        let entry = match self.cached_modules.entry(code_hash) {
            Entry::Occupied(entry) => {
                return entry.into_mut();
            }
            Entry::Vacant(entry) => entry,
        };

        let (address, rwasm_module) = match bytecode_or_hash {
            BytecodeOrHash::Bytecode {
                address,
                bytecode: rwasm_module,
                ..
            } => (address, rwasm_module),
            BytecodeOrHash::Hash(_hash) => {
                panic!("runtime: can't compile just by hash")
            }
        };

        println!("missing strategy: code_hash={code_hash} address={address}");

        let _span = tracing::info_span!("parse_rwasm_module").entered();
        let rwasm_module = Rc::new(RwasmModule::new_or_empty(rwasm_module.as_ref()).0);
        drop(_span);

        #[cfg(feature = "wasmtime")]
        if fluentbase_types::is_system_precompile(&address)
            && rwasm_module.hint_type() == rwasm::HintType::WASM
        {
            return Self::init_wasmtime(self.import_linker.clone(), entry, rwasm_module, code_hash);
        }

        Self::init_rwasm(self.import_linker.clone(), entry, rwasm_module, code_hash)
    }

    #[cfg(feature = "wasmtime")]
    #[tracing::instrument(level = "info", skip_all, fields(code_hash = %code_hash))]
    fn init_wasmtime(
        import_linker: Rc<ImportLinker>,
        entry: VacantEntry<B256, CachedModule>,
        rwasm_module: Rc<RwasmModule>,
        #[allow(unused)] code_hash: B256,
    ) -> &mut CachedModule {
        // The lock helps to avoid recompiling same wasmtime modules,
        // if you run multiple tests in parallel
        #[cfg(feature = "inter-process-lock")]
        let _guard = crate::inter_process_lock::InterProcessLock::acquire(
            crate::inter_process_lock::FILE_NAME_PREFIX1,
            code_hash.to_string(),
        )
        .unwrap();

        let config = fluentbase_types::default_compilation_config().with_consume_fuel(false);

        // Theoretically, it should always compile, because we compile in advance on node start and node just won't start,
        // if it wasmtime modules can't be compiled (all runtimes, like evm, svm, wasm, etc.)
        let wasmtime_module = rwasm::compile_wasmtime_module(config, &rwasm_module.hint_section)
            .expect("failed to compile wasmtime module, this should never happen");

        let strategy = Strategy::Wasmtime {
            module: Rc::new(wasmtime_module),
        };
        let cached_module = CachedModule::new(strategy, import_linker);
        entry.insert(cached_module)
    }

    #[tracing::instrument(level = "info", skip_all, fields(code_hash = %code_hash))]
    fn init_rwasm(
        import_linker: Rc<ImportLinker>,
        entry: VacantEntry<B256, CachedModule>,
        rwasm_module: Rc<RwasmModule>,
        #[allow(unused)] code_hash: B256,
    ) -> &mut CachedModule {
        let strategy = Strategy::Rwasm {
            module: rwasm_module,
            engine: ExecutionEngine::acquire_shared(),
        };
        let cached_module = CachedModule::new(strategy, import_linker);
        entry.insert(cached_module)
    }
}
