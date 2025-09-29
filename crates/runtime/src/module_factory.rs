use fluentbase_types::{BytecodeOrHash, HashMap, B256};
use rwasm::RwasmModule;
use std::sync::{Arc, LazyLock, RwLock};

/// Global factory maintaining compiled module cache and resumable runtime instances.
#[derive(Clone)]
pub struct ModuleFactory {
    inner: Arc<RwLock<ModuleFactoryInner>>,
}

impl ModuleFactory {
    /// Creates a factory configured for the v1 preview import surface.
    pub fn new() -> Self {
        static INSTANCE: LazyLock<ModuleFactory> = LazyLock::new(|| ModuleFactory {
            inner: Arc::new(RwLock::new(ModuleFactoryInner::default())),
        });
        INSTANCE.clone()
    }

    /// Returns a cached module for the given bytecode or compiles and caches it on first use.
    pub fn get_module_or_init(&mut self, bytecode_or_hash: BytecodeOrHash) -> RwasmModule {
        let code_hash = bytecode_or_hash.code_hash();
        let ctx = self.inner.read().unwrap();
        if let Some(entry) = ctx.cached_modules.get(&code_hash) {
            return entry.clone();
        }
        drop(ctx);
        let mut ctx = self.inner.write().unwrap();
        if let Some(entry) = ctx.cached_modules.get(&code_hash) {
            return entry.clone();
        }
        let rwasm_module = match bytecode_or_hash {
            BytecodeOrHash::Bytecode { bytecode, .. } => bytecode,
            BytecodeOrHash::Hash(_hash) => {
                // TODO(dmitry123): Do we want to have lock here until resources are warmed up?
                panic!("runtime: can't compile just by hash")
            }
        };
        ctx.cached_modules.insert(code_hash, rwasm_module.clone());
        rwasm_module
    }

    #[cfg(feature = "wasmtime")]
    pub fn get_wasmtime_module_or_compile(
        &mut self,
        code_hash: B256,
        address: fluentbase_types::Address,
    ) -> wasmtime::Module {
        let ctx = self.inner.read().unwrap();
        if let Some(module) = ctx.wasmtime_modules.get(&code_hash) {
            return module.clone();
        }
        drop(ctx);
        let mut ctx = self.inner.write().unwrap();
        if let Some(module) = ctx.wasmtime_modules.get(&code_hash) {
            return module.clone();
        }
        println!(
            "missing wasmtime module address={} code_hash={}, compiling",
            address, code_hash
        );
        let rwasm_module = ctx
            .cached_modules
            .get(&code_hash)
            .cloned()
            .expect("runtime: missing rwasm module during wasmtime compilation");
        use rwasm::{compile_wasmtime_module, CompilationConfig};
        let module =
            compile_wasmtime_module(CompilationConfig::default(), &rwasm_module.hint_section)
                .unwrap();
        ctx.wasmtime_modules.insert(code_hash, module.clone());
        module
    }

    #[cfg(feature = "wasmtime")]
    pub fn warmup_wasmtime(
        &mut self,
        rwasm_module: RwasmModule,
        wasmtime_module: wasmtime::Module,
        code_hash: B256,
    ) {
        let mut ctx = self.inner.write().unwrap();
        ctx.cached_modules.insert(code_hash, rwasm_module);
        ctx.wasmtime_modules
            .insert(code_hash, wasmtime_module.clone());
    }
}

#[derive(Default)]
struct ModuleFactoryInner {
    /// Cache of compiled modules keyed by code hash.
    /// TODO(dmitry123): Add LRU cache to this map to avoid memory leak
    pub cached_modules: HashMap<B256, RwasmModule>,
    /// Cache of wasmtime modules
    #[cfg(feature = "wasmtime")]
    pub wasmtime_modules: HashMap<B256, wasmtime::Module>,
}
