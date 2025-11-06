use fluentbase_types::{BytecodeOrHash, B256};
use rwasm::RwasmModule;
use schnellru::{Limiter, LruMap};
use std::sync::{Arc, LazyLock, Mutex};

/// Global factory maintaining compiled module cache and resumable runtime instances.
#[derive(Clone)]
pub struct ModuleFactory {
    inner: Arc<Mutex<ModuleFactoryInner>>,
}

impl ModuleFactory {
    /// Creates a factory configured for the v1 preview import surface.
    pub fn new() -> Self {
        static INSTANCE: LazyLock<ModuleFactory> = LazyLock::new(|| ModuleFactory {
            inner: Arc::new(Mutex::new(ModuleFactoryInner::default())),
        });
        INSTANCE.clone()
    }

    /// Returns a cached module for the given bytecode or compiles and caches it on first use.
    pub fn get_module_or_init(&mut self, bytecode_or_hash: BytecodeOrHash) -> RwasmModule {
        let code_hash = bytecode_or_hash.code_hash();
        let mut ctx = self.inner.lock().unwrap();

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
        let mut ctx = self.inner.lock().unwrap();

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
        let mut ctx = self.inner.lock().unwrap();
        ctx.cached_modules.insert(code_hash, rwasm_module);
        ctx.wasmtime_modules
            .insert(code_hash, wasmtime_module.clone());
    }
}

struct ModuleFactoryInner {
    pub cached_modules: LruMap<B256, RwasmModule, ModuleMemoryLimiter<RwasmModule>>,
    #[cfg(feature = "wasmtime")]
    pub wasmtime_modules: LruMap<B256, wasmtime::Module, ModuleMemoryLimiter<wasmtime::Module>>,
}

/// Maximum memory for module cache: 1 GB
pub const CACHED_MODULES_SIZE_LIMIT: usize = 1024 * 1024 * 1024;

impl Default for ModuleFactoryInner {
    fn default() -> Self {
        Self {
            cached_modules: LruMap::new(ModuleMemoryLimiter::<RwasmModule>::new(
                CACHED_MODULES_SIZE_LIMIT,
            )),
            #[cfg(feature = "wasmtime")]
            wasmtime_modules: LruMap::new(ModuleMemoryLimiter::<wasmtime::Module>::new(
                CACHED_MODULES_SIZE_LIMIT,
            )),
        }
    }
}

pub trait SizeEstimator {
    fn estimate_size(&self) -> usize;
}

impl SizeEstimator for RwasmModule {
    /// Estimates memory size by calculating heap-allocated section sizes.
    ///
    /// Formula:
    /// - code_section: instructions × 8 bytes per Opcode
    /// - hint_section: original bytecode bytes
    /// - data_section: static data bytes
    /// - elem_section: elements × 4 bytes per u32
    #[inline]
    fn estimate_size(&self) -> usize {
        const OPCODE_SIZE: usize = 8;
        const U32_SIZE: usize = 4;

        self.code_section.len() * OPCODE_SIZE
            + self.hint_section.len()
            + self.data_section.len()
            + self.elem_section.len() * U32_SIZE
    }
}

#[cfg(feature = "wasmtime")]
impl SizeEstimator for wasmtime::Module {
    /// Estimates memory size using the compiled artifact's image range.
    ///
    /// This includes executable code, data sections, and metadata.
    #[inline]
    fn estimate_size(&self) -> usize {
        let range = self.image_range();
        let start = range.start as usize;
        let end = range.end as usize;
        end - start
    }
}

#[derive(Clone, Debug)]
pub struct ModuleMemoryLimiter<V> {
    max_bytes: usize,
    current_bytes: usize,
    _marker: std::marker::PhantomData<V>,
}

impl<V> ModuleMemoryLimiter<V> {
    pub const fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            current_bytes: 0,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<V: SizeEstimator> Limiter<B256, V> for ModuleMemoryLimiter<V> {
    type KeyToInsert<'a> = B256;
    type LinkType = u32;

    #[inline]
    fn is_over_the_limit(&self, _length: usize) -> bool {
        self.current_bytes > self.max_bytes
    }

    fn on_insert(
        &mut self,
        _length: usize,
        key: Self::KeyToInsert<'_>,
        value: V,
    ) -> Option<(B256, V)> {
        let size = value.estimate_size();

        if size > self.max_bytes {
            return None;
        }

        self.current_bytes += size;
        Some((key, value))
    }

    fn on_replace(
        &mut self,
        _length: usize,
        _old_key: &mut B256,
        _new_key: Self::KeyToInsert<'_>,
        old_value: &mut V,
        new_value: &mut V,
    ) -> bool {
        let old_size = old_value.estimate_size();
        let new_size = new_value.estimate_size();

        if new_size > old_size {
            let diff = new_size - old_size;
            if self.current_bytes + diff > self.max_bytes {
                return false;
            }
            self.current_bytes += diff;
        } else {
            self.current_bytes = self.current_bytes.saturating_sub(old_size - new_size);
        }

        true
    }

    fn on_removed(&mut self, _key: &mut B256, value: &mut V) {
        let size = value.estimate_size();
        self.current_bytes = self.current_bytes.saturating_sub(size);
    }

    fn on_cleared(&mut self) {
        self.current_bytes = 0;
    }

    fn on_grow(&mut self, new_memory_usage: usize) -> bool {
        new_memory_usage <= self.max_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rwasm::{InstructionSet, RwasmModuleInner};

    fn create_rwasm_module(hint_size: usize, hint_value: u8) -> RwasmModule {
        RwasmModuleInner {
            code_section: InstructionSet::default(),
            hint_section: vec![hint_value; hint_size],
            data_section: vec![],
            elem_section: vec![],
        }
        .into()
    }

    #[test]
    fn enforces_memory_limit() {
        const LIMIT: usize = 5 * 1024; // 5 KB
        let mut cache = LruMap::new(ModuleMemoryLimiter::<RwasmModule>::new(LIMIT));

        // Insert 10 modules × 1 KB = 10 KB (2x over limit)
        for i in 0..10u8 {
            cache.insert(B256::from([i; 32]), create_rwasm_module(1024, i));
        }

        // Should evict old entries to stay near limit
        assert!(cache.len() == 5, "Should evict to stay under limit");
        assert_eq!(
            cache.len() * 1024,
            LIMIT,
            "Memory usage should be close to limit"
        );
        assert_eq!(
            cache.get(&B256::from([1; 32])),
            None,
            "Oldest item should be evicted"
        );
        assert_eq!(
            cache.get(&B256::from([5; 32])),
            Some(&mut create_rwasm_module(1024, 5)),
            "Recent item should remain"
        );
    }

    #[test]
    fn access_updates_lru_order() {
        const LIMIT: usize = 3 * 1024;
        let mut cache = LruMap::with_seed(ModuleMemoryLimiter::<RwasmModule>::new(LIMIT), [0; 4]);

        // Insert 3 modules (at capacity)
        cache.insert(B256::from([0; 32]), create_rwasm_module(1024, 0));
        cache.insert(B256::from([1; 32]), create_rwasm_module(1024, 1));
        cache.insert(B256::from([2; 32]), create_rwasm_module(1024, 2));

        // Access oldest item (makes it newest)
        let _ = cache.get(&B256::from([0; 32]));

        // Insert new item (should evict [1], not [0])
        cache.insert(B256::from([3; 32]), create_rwasm_module(1024, 3));

        assert!(
            cache.get(&B256::from([0; 32])).is_some(),
            "Accessed item should remain"
        );
        assert!(
            cache.get(&B256::from([1; 32])).is_none(),
            "Non-accessed old item should be evicted"
        );
    }
}
