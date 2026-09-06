use fluentbase_types::{
    BytecodeOrHash, CompilationBackend, CompilationConfigFingerprint, CompiledModuleCacheKey,
    ExitCode, B256,
};
use rwasm::RwasmModule;
use schnellru::{Limiter, LruMap};
use std::{
    collections::HashMap,
    marker::PhantomData,
    sync::{Arc, LazyLock, Mutex},
};

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
    /// Hash-only lookups require a resident module from an earlier bytecode warmup.
    pub fn get_module_or_init(
        &mut self,
        bytecode_or_hash: BytecodeOrHash,
    ) -> Result<RwasmModule, ExitCode> {
        let mut ctx = self.inner.lock().unwrap();
        let code_hash = bytecode_or_hash.code_hash();
        let module_key = match &bytecode_or_hash {
            BytecodeOrHash::Bytecode { address, .. } => CompiledModuleCacheKey::new(
                code_hash,
                CompilationConfigFingerprint::from_config(
                    &fluentbase_sdk_config_for_runtime_cache(*address),
                    CompilationBackend::Rwasm,
                    *address,
                ),
            ),
            BytecodeOrHash::Hash(_hash) => {
                // Hash-only lookups are only valid after an earlier bytecode warmup. Keep this
                // deterministic by resolving through the explicit code-hash index.
                ctx.cached_modules
                    .limiter()
                    .module_keys_by_code_hash
                    .get(&code_hash)
                    .copied()
                    .ok_or(ExitCode::UnknownError)?
            }
        };

        if let Some(entry) = ctx.cached_modules.get(&module_key) {
            return Ok(entry.clone());
        }

        let rwasm_module = match bytecode_or_hash {
            BytecodeOrHash::Bytecode { bytecode, .. } => bytecode,
            BytecodeOrHash::Hash(_) => return Err(ExitCode::UnknownError),
        };

        if !ctx.cached_modules.insert(module_key, rwasm_module.clone())
            && ctx.cached_modules.is_empty()
        {
            // An initial table allocation can fail after on_insert without an on_removed
            // callback. Clearing also rolls back that tentative index entry and byte charge.
            ctx.cached_modules.clear();
        }
        Ok(rwasm_module)
    }
}

struct ModuleFactoryInner {
    pub cached_modules:
        LruMap<CompiledModuleCacheKey, RwasmModule, ModuleMemoryLimiter<RwasmModule>>,
}

/// Maximum memory for module cache: 1 GB
///
/// This limits only the estimated size of cached module content,
/// not the hash table overhead (which is negligible for typical workloads).
/// The code-hash index contains at most one entry per resident module; its allocation
/// is bounded by peak cache residency rather than lifetime deployment count.
pub const CACHED_MODULES_SIZE_LIMIT: usize = 1024 * 1024 * 1024;

impl Default for ModuleFactoryInner {
    fn default() -> Self {
        Self {
            cached_modules: LruMap::new(ModuleMemoryLimiter::<RwasmModule>::new(
                CACHED_MODULES_SIZE_LIMIT,
            )),
        }
    }
}

fn fluentbase_sdk_config_for_runtime_cache(
    address: fluentbase_types::Address,
) -> rwasm::CompilationConfig {
    let is_system_runtime = fluentbase_types::is_execute_using_system_runtime(&address);
    let should_charge_fuel = false;

    fluentbase_sdk_like_default_config()
        .with_consume_fuel(should_charge_fuel)
        .with_consume_fuel_for_bulk_ops(!is_system_runtime)
        .with_consume_fuel_for_params_and_locals(!is_system_runtime)
        .with_builtins_consume_fuel(should_charge_fuel)
        .with_max_allowed_memory_pages(if is_system_runtime {
            rwasm::N_MAX_ALLOWED_MEMORY_PAGES
        } else {
            rwasm::N_DEFAULT_MAX_MEMORY_PAGES
        })
        .with_allow_malformed_entrypoint_func_type(is_system_runtime)
}

fn fluentbase_sdk_like_default_config() -> rwasm::CompilationConfig {
    rwasm::CompilationConfig::default()
        .with_state_router(rwasm::StateRouterConfig {
            states: Box::new([
                ("deploy".into(), fluentbase_types::STATE_DEPLOY),
                ("main".into(), fluentbase_types::STATE_MAIN),
            ]),
            opcode: Some(rwasm::Opcode::Call(
                fluentbase_types::SysFuncIdx::STATE as u32,
            )),
        })
        .with_import_linker(fluentbase_types::import_linker_v1_preview())
        .with_allow_malformed_entrypoint_func_type(false)
        .with_consume_fuel_for_bulk_ops(true)
        .with_builtins_consume_fuel(true)
}

/// Trait for estimating heap-allocated memory size of cached values.
pub trait SizeEstimator {
    /// Returns estimated heap memory usage in bytes.
    ///
    /// Must return a value > 0 for valid entries. Zero-size entries
    /// are rejected by the limiter to prevent unbounded cache growth.
    fn estimate_size(&self) -> usize;
}

impl SizeEstimator for RwasmModule {
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

/// Memory-based limiter for LRU cache that tracks total byte usage.
///
/// Evicts least-recently-used entries when total cached size exceeds `max_bytes`.
///
/// # Rejection Rules
/// - Items with `estimate_size() == 0` are rejected (would bypass limits)
/// - Items with `estimate_size() > max_bytes` are rejected (can never fit)
///
/// # Example
/// ```ignore
/// let limiter = ModuleMemoryLimiter::<MyModule>::new(1024 * 1024); // 1MB limit
/// let mut cache = LruMap::new(limiter);
/// ```
#[derive(Clone, Debug)]
pub struct ModuleMemoryLimiter<V> {
    max_bytes: usize,
    current_bytes: usize,
    // Owned by the limiter so all LRU removal paths prune the index under the factory lock.
    module_keys_by_code_hash: HashMap<B256, CompiledModuleCacheKey>,
    _marker: PhantomData<V>,
}

impl<V> ModuleMemoryLimiter<V> {
    /// Creates a new limiter with the specified memory budget.
    ///
    /// # Panics
    /// Panics if `max_bytes` is 0 (would reject all entries).
    pub fn new(max_bytes: usize) -> Self {
        assert!(max_bytes > 0, "max_bytes must be greater than 0");
        Self {
            max_bytes,
            current_bytes: 0,
            module_keys_by_code_hash: HashMap::new(),
            _marker: PhantomData,
        }
    }

    /// Returns the maximum memory budget in bytes.
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Returns current tracked memory usage in bytes.
    pub const fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    /// Returns remaining available memory in bytes.
    pub const fn available_bytes(&self) -> usize {
        self.max_bytes.saturating_sub(self.current_bytes)
    }
}

impl<V: SizeEstimator> Limiter<CompiledModuleCacheKey, V> for ModuleMemoryLimiter<V> {
    type KeyToInsert<'a> = CompiledModuleCacheKey;
    type LinkType = u32;

    /// Checks if eviction is needed after an insert or replacement.
    #[inline]
    fn is_over_the_limit(&self, _length: usize) -> bool {
        self.current_bytes > self.max_bytes
    }

    /// Validates and tracks a new entry before insertion.
    ///
    /// Returns `None` if:
    /// - `estimate_size() == 0` (zero-size items bypass limits)
    /// - `estimate_size() > max_bytes` (item can never fit)
    fn on_insert(
        &mut self,
        _length: usize,
        key: Self::KeyToInsert<'_>,
        value: V,
    ) -> Option<(CompiledModuleCacheKey, V)> {
        let size = value.estimate_size();

        if size == 0 || size > self.max_bytes {
            return None;
        }

        self.current_bytes = self.current_bytes.saturating_add(size);
        self.module_keys_by_code_hash.insert(key.code_hash, key);
        Some((key, value))
    }

    /// Validates and tracks a value replacement for an existing key.
    ///
    /// Returns `false` (causing entry removal) if:
    /// - `new_value.estimate_size() == 0`
    /// - `new_value.estimate_size() > max_bytes`
    ///
    /// Otherwise, updates size tracking and returns `true`, allowing LRU
    /// eviction to handle any overflow.
    fn on_replace(
        &mut self,
        _length: usize,
        old_key: &mut CompiledModuleCacheKey,
        _new_key: Self::KeyToInsert<'_>,
        old_value: &mut V,
        new_value: &mut V,
    ) -> bool {
        let old_size = old_value.estimate_size();
        let new_size = new_value.estimate_size();

        if new_size == 0 || new_size > self.max_bytes {
            return false;
        }

        self.current_bytes = self
            .current_bytes
            .saturating_sub(old_size)
            .saturating_add(new_size);
        self.module_keys_by_code_hash
            .insert(old_key.code_hash, *old_key);

        true
    }

    /// Updates size tracking after an entry is removed.
    fn on_removed(&mut self, key: &mut CompiledModuleCacheKey, value: &mut V) {
        let size = value.estimate_size();
        self.current_bytes = self.current_bytes.saturating_sub(size);
        // The same code hash can have multiple compilation profiles. Removing an older
        // profile must not discard the index entry for a newer, still-resident profile.
        if self.module_keys_by_code_hash.get(&key.code_hash) == Some(key) {
            self.module_keys_by_code_hash.remove(&key.code_hash);
        }
    }

    /// Resets size tracking when the cache is cleared.
    fn on_cleared(&mut self) {
        self.current_bytes = 0;
        self.module_keys_by_code_hash.clear();
    }

    /// Controls whether the internal hash table can grow its bucket array.
    ///
    /// The `new_memory_usage` parameter is the allocation size for the table's
    /// internal structure (entry slots + control bytes), NOT the stored content.
    ///
    /// Always returns `true`: we budget content size only, not table overhead.
    fn on_grow(&mut self, _new_memory_usage: usize) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rwasm::{InstructionSet, RwasmModuleInner};

    // ==================== Test Helpers ====================

    /// Creates a module with specified hint_section size.
    fn module(hint_size: usize) -> RwasmModule {
        RwasmModuleInner {
            code_section: InstructionSet::default(),
            hint_section: vec![0u8; hint_size],
            data_section: vec![],
            elem_section: vec![],
            source_pc: 0,
        }
        .into()
    }

    /// Creates a deterministic B256 key from a u16 id.
    fn key(id: u16) -> B256 {
        let mut bytes = [0u8; 32];
        bytes[0..2].copy_from_slice(&id.to_le_bytes());
        B256::from(bytes)
    }

    /// Fixed seed for deterministic hash table behavior.
    const TEST_SEED: [u64; 4] = [1, 2, 3, 4];

    fn cache_key(id: u16) -> CompiledModuleCacheKey {
        cache_key_with_address_byte(key(id), id as u8)
    }

    fn cache_key_with_address_byte(code_hash: B256, address_byte: u8) -> CompiledModuleCacheKey {
        CompiledModuleCacheKey::new(
            code_hash,
            CompilationConfigFingerprint::from_config(
                &fluentbase_sdk_like_default_config(),
                CompilationBackend::Rwasm,
                fluentbase_types::Address::repeat_byte(address_byte),
            ),
        )
    }

    fn new_cache(
        max_bytes: usize,
    ) -> LruMap<CompiledModuleCacheKey, RwasmModule, ModuleMemoryLimiter<RwasmModule>> {
        LruMap::with_seed(ModuleMemoryLimiter::new(max_bytes), TEST_SEED)
    }

    #[test]
    fn factory_index_is_bounded_under_deployment_churn() {
        let mut factory = ModuleFactory {
            inner: Arc::new(Mutex::new(ModuleFactoryInner {
                cached_modules: new_cache(500),
            })),
        };

        for id in 0..1000 {
            factory
                .get_module_or_init(BytecodeOrHash::Bytecode {
                    bytecode: module(100),
                    hash: key(id),
                    address: fluentbase_types::Address::ZERO,
                })
                .unwrap();
            let ctx = factory.inner.lock().unwrap();
            assert!(ctx.cached_modules.len() <= 5);
            assert_eq!(
                ctx.cached_modules.limiter().module_keys_by_code_hash.len(),
                ctx.cached_modules.len()
            );
        }
    }

    #[test]
    fn hash_only_misses_do_not_poison_factory_after_eviction() {
        let mut factory = ModuleFactory {
            inner: Arc::new(Mutex::new(ModuleFactoryInner {
                cached_modules: new_cache(100),
            })),
        };

        assert!(matches!(
            factory.get_module_or_init(BytecodeOrHash::Hash(key(1))),
            Err(ExitCode::UnknownError)
        ));
        for id in [1, 2, 1] {
            factory
                .get_module_or_init(BytecodeOrHash::Bytecode {
                    bytecode: module(100),
                    hash: key(id),
                    address: fluentbase_types::Address::ZERO,
                })
                .unwrap();
            let cached = factory
                .get_module_or_init(BytecodeOrHash::Hash(key(id)))
                .unwrap();
            assert_eq!(cached.hint_section.len(), 100);
            assert!(matches!(
                factory.get_module_or_init(BytecodeOrHash::Hash(key(3 - id))),
                Err(ExitCode::UnknownError)
            ));
        }
        assert!(!factory.inner.is_poisoned());
    }

    #[test]
    fn rejected_modules_are_returned_without_index_entries() {
        let mut factory = ModuleFactory {
            inner: Arc::new(Mutex::new(ModuleFactoryInner {
                cached_modules: new_cache(100),
            })),
        };

        for hint_size in [0, 101] {
            let uncached = factory
                .get_module_or_init(BytecodeOrHash::Bytecode {
                    bytecode: module(hint_size),
                    hash: key(1),
                    address: fluentbase_types::Address::ZERO,
                })
                .unwrap();
            assert_eq!(uncached.hint_section.len(), hint_size);
            let ctx = factory.inner.lock().unwrap();
            assert!(ctx.cached_modules.is_empty());
            assert!(ctx
                .cached_modules
                .limiter()
                .module_keys_by_code_hash
                .is_empty());
        }
    }

    #[test]
    fn evicting_older_profile_preserves_newer_hash_mapping() {
        let mut cache = new_cache(200);
        let code_hash = key(42);
        let older = cache_key_with_address_byte(code_hash, 0xaa);
        let newer = cache_key_with_address_byte(code_hash, 0xbb);

        cache.insert(older, module(100));
        cache.insert(newer, module(100));
        cache.insert(cache_key(1), module(100));

        assert!(cache.get(&older).is_none());
        assert!(cache.get(&newer).is_some());
        assert_eq!(
            cache.limiter().module_keys_by_code_hash.get(&code_hash),
            Some(&newer)
        );
        cache.remove(&newer);
        assert!(!cache
            .limiter()
            .module_keys_by_code_hash
            .contains_key(&code_hash));
    }

    #[test]
    fn replacement_restores_hash_mapping_after_another_profile_is_removed() {
        let mut cache = new_cache(200);
        let code_hash = key(42);
        let key_a = cache_key_with_address_byte(code_hash, 0xaa);
        let key_b = cache_key_with_address_byte(code_hash, 0xbb);

        cache.insert(key_a, module(100));
        cache.insert(key_b, module(100));
        cache.remove(&key_b);
        assert!(!cache
            .limiter()
            .module_keys_by_code_hash
            .contains_key(&code_hash));

        cache.insert(key_a, module(50));
        assert_eq!(
            cache.limiter().module_keys_by_code_hash.get(&code_hash),
            Some(&key_a)
        );
    }

    // ==================== Basic Operations ====================

    #[test]
    fn insert_and_retrieve() {
        let mut cache = new_cache(1000);

        cache.insert(cache_key(1), module(100));

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.limiter().current_bytes(), 100);
        assert!(cache.get(&cache_key(1)).is_some());
    }

    #[test]
    fn identical_code_hash_with_different_fingerprints_uses_distinct_entries() {
        let mut cache = new_cache(1000);
        let code_hash = key(42);
        let key_a = cache_key_with_address_byte(code_hash, 0xaa);
        let key_b = cache_key_with_address_byte(code_hash, 0xbb);

        cache.insert(key_a, module(100));
        cache.insert(key_b, module(100));

        assert_ne!(key_a.config_fingerprint, key_b.config_fingerprint);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&key_a).is_some());
        assert!(cache.get(&key_b).is_some());
    }

    #[test]
    fn remove_updates_tracking() {
        let mut cache = new_cache(1000);

        cache.insert(cache_key(1), module(100));
        cache.remove(&cache_key(1));

        assert_eq!(cache.len(), 0);
        assert_eq!(cache.limiter().current_bytes(), 0);
    }

    #[test]
    fn clear_resets_tracking() {
        let mut cache = new_cache(1000);

        cache.insert(cache_key(1), module(100));
        cache.insert(cache_key(2), module(200));
        cache.clear();

        assert_eq!(cache.len(), 0);
        assert_eq!(cache.limiter().current_bytes(), 0);
        assert!(cache.limiter().module_keys_by_code_hash.is_empty());
    }

    // ==================== LRU Eviction ====================

    #[test]
    fn evicts_lru_when_over_limit() {
        let mut cache = new_cache(500); // 5 × 100 bytes

        for i in 0..10u16 {
            cache.insert(cache_key(i), module(100));
        }

        assert_eq!(cache.len(), 5);
        assert_eq!(cache.limiter().current_bytes(), 500);

        // Oldest (0-4) evicted, newest (5-9) remain
        for i in 0..5u16 {
            assert!(
                cache.get(&cache_key(i)).is_none(),
                "key({i}) should be evicted"
            );
        }
        for i in 5..10u16 {
            assert!(cache.get(&cache_key(i)).is_some(), "key({i}) should remain");
        }
    }

    #[test]
    fn access_promotes_entry() {
        let mut cache = new_cache(300); // 3 × 100 bytes

        cache.insert(cache_key(0), module(100));
        cache.insert(cache_key(1), module(100));
        cache.insert(cache_key(2), module(100));

        // Promote key(0) to MRU
        let _ = cache.get(&cache_key(0));

        // Insert key(3) → evicts LRU (key(1))
        cache.insert(cache_key(3), module(100));

        assert!(
            cache.get(&cache_key(0)).is_some(),
            "accessed entry should remain"
        );
        assert!(cache.get(&cache_key(1)).is_none(), "LRU should be evicted");
        assert!(cache.get(&cache_key(2)).is_some());
        assert!(cache.get(&cache_key(3)).is_some());
    }

    // ==================== Replacement Behavior ====================

    #[test]
    fn replacement_triggers_eviction_not_removal() {
        let mut cache = new_cache(300); // 3 × 100 bytes

        cache.insert(cache_key(1), module(100));
        cache.insert(cache_key(2), module(100));
        cache.insert(cache_key(3), module(100));

        // Replace key(2): 100 → 150, total 350 → evicts key(1)
        cache.insert(cache_key(2), module(150));

        assert!(
            cache.get(&cache_key(2)).is_some(),
            "replaced entry must remain"
        );
        assert!(cache.get(&cache_key(1)).is_none(), "LRU should be evicted");
        assert!(cache.get(&cache_key(3)).is_some());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.limiter().current_bytes(), 250);
    }

    #[test]
    fn replacement_with_smaller_value() {
        let mut cache = new_cache(200);

        cache.insert(cache_key(1), module(150));
        cache.insert(cache_key(1), module(50));

        assert_eq!(cache.limiter().current_bytes(), 50);
        assert_eq!(cache.limiter().available_bytes(), 150);
    }

    // ==================== Rejection Cases ====================

    #[test]
    fn rejects_oversized_item() {
        let mut cache = new_cache(100);

        let inserted = cache.insert(cache_key(1), module(101));

        assert!(!inserted);
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.limiter().current_bytes(), 0);
    }

    #[test]
    fn rejects_zero_size_item() {
        let mut cache = new_cache(100);

        let inserted = cache.insert(cache_key(1), module(0));

        assert!(!inserted);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn replacement_with_oversized_removes_entry() {
        let mut cache = new_cache(100);

        cache.insert(cache_key(1), module(50));
        cache.insert(cache_key(1), module(101));

        assert_eq!(cache.len(), 0);
        assert_eq!(cache.limiter().current_bytes(), 0);
        assert!(cache.limiter().module_keys_by_code_hash.is_empty());
    }

    #[test]
    fn replacement_with_zero_size_removes_entry() {
        let mut cache = new_cache(100);

        cache.insert(cache_key(1), module(50));
        cache.insert(cache_key(1), module(0));

        assert_eq!(cache.len(), 0);
        assert_eq!(cache.limiter().current_bytes(), 0);
        assert!(cache.limiter().module_keys_by_code_hash.is_empty());
    }

    // ==================== Limiter Construction ====================

    #[test]
    #[should_panic(expected = "max_bytes must be greater than 0")]
    fn panics_on_zero_max_bytes() {
        ModuleMemoryLimiter::<RwasmModule>::new(0);
    }

    // ==================== Edge Cases ====================

    #[test]
    fn exact_capacity_fit() {
        let mut cache = new_cache(100);

        let inserted = cache.insert(cache_key(1), module(100));

        assert!(inserted);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.limiter().current_bytes(), 100);
        assert_eq!(cache.limiter().available_bytes(), 0);
    }

    #[test]
    fn multiple_evictions_for_large_item() {
        let mut cache = new_cache(400); // 4 × 100 bytes

        for i in 0..4u16 {
            cache.insert(cache_key(i), module(100));
        }
        assert_eq!(cache.len(), 4);

        // Insert 250 bytes → evicts 3 items to fit
        cache.insert(cache_key(10), module(250));

        assert_eq!(cache.len(), 2);
        assert!(cache.limiter().current_bytes() <= 400);
        assert!(cache.get(&cache_key(10)).is_some());
    }

    #[test]
    fn hash_table_growth() {
        let mut cache = new_cache(100_000);

        for i in 0..500u16 {
            cache.insert(cache_key(i), module(100));
        }

        assert_eq!(cache.len(), 500);
        assert_eq!(cache.limiter().current_bytes(), 50_000);

        // Verify access after growth
        assert!(cache.get(&cache_key(0)).is_some());
        assert!(cache.get(&cache_key(250)).is_some());
        assert!(cache.get(&cache_key(499)).is_some());
    }
}
