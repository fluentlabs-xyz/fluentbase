use fluentbase_types::{
    Address, CompilationBackend, CompilationConfigFingerprint, CompiledModuleCacheKey, B256,
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

    /// Returns the cached module for `bytecode`, inserting it on first use.
    ///
    /// Always yields a module: when the memory limiter rejects the entry, the supplied module is
    /// returned without being cached.
    pub fn get_or_insert_module(
        &mut self,
        bytecode: RwasmModule,
        hash: B256,
        address: Address,
    ) -> RwasmModule {
        let module_key = module_key_for(hash, address);
        self.inner
            .lock()
            .unwrap()
            .get_or_insert(module_key, bytecode)
    }

    /// Returns the resident module for `code_hash`, promoting it in the LRU.
    ///
    /// Yields `None` when no module with this code hash is resident: none was ever supplied
    /// through [`Self::get_or_insert_module`], or the memory limiter evicted it since.
    pub fn get_resident_module(&mut self, code_hash: B256) -> Option<RwasmModule> {
        self.inner.lock().unwrap().get_resident(code_hash)
    }
}

struct ModuleFactoryInner {
    pub cached_modules:
        LruMap<CompiledModuleCacheKey, RwasmModule, ModuleMemoryLimiter<RwasmModule>>,
}

impl ModuleFactoryInner {
    /// Returns the resident module that hash-only lookups resolve to for `code_hash`.
    fn get_resident(&mut self, code_hash: B256) -> Option<RwasmModule> {
        let module_key = self.cached_modules.limiter().resident_key(&code_hash)?;
        // The index is written by `on_insert` and pruned on every removal path (`on_removed`,
        // `on_cleared`, and the failed-insert rollback in `get_or_insert`), so a hit names a
        // resident entry and `get` only promotes it.
        self.cached_modules.get(&module_key).cloned()
    }

    /// Returns the cached module under `module_key`, inserting `module` on first use.
    fn get_or_insert(
        &mut self,
        module_key: CompiledModuleCacheKey,
        module: RwasmModule,
    ) -> RwasmModule {
        if let Some(entry) = self.cached_modules.get(&module_key) {
            return entry.clone();
        }
        if !self.cached_modules.insert(module_key, module.clone()) {
            // `on_insert` indexes the key and charges its size before the table insert, which
            // can still fail on a table allocation error without an `on_removed` callback. Drop
            // the tentative index entry so the index keeps mirroring residency; a rejected entry
            // was never indexed, so this is a no-op for it.
            self.cached_modules.limiter_mut().forget_key(&module_key);
            if self.cached_modules.is_empty() {
                // Nothing is resident, so the byte charge must be zero as well.
                self.cached_modules.clear();
            }
        }
        module
    }
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

/// Derives the cache key for `hash` compiled under the profile of `address`.
///
/// The fingerprint covers the address itself, so the same code deployed at several addresses
/// occupies one entry per address.
fn module_key_for(hash: B256, address: Address) -> CompiledModuleCacheKey {
    CompiledModuleCacheKey::new(
        hash,
        CompilationConfigFingerprint::from_config(
            &fluentbase_sdk_config_for_runtime_cache(address),
            CompilationBackend::Rwasm,
            address,
        ),
    )
}

/// Returns the compilation config that modules executed at `address` are cached under.
fn fluentbase_sdk_config_for_runtime_cache(address: Address) -> rwasm::CompilationConfig {
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
    /// Code hash to the resident profiles that share it, oldest insertion first.
    ///
    /// Hash-only lookups resolve to the last entry: the most recently inserted or replaced
    /// profile. Owned by the limiter so every LRU removal path, including eviction inside
    /// `insert`, prunes the index under the factory lock.
    module_keys_by_code_hash: HashMap<B256, Vec<CompiledModuleCacheKey>>,
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

    /// Returns the cache key that hash-only lookups resolve to for `code_hash`.
    fn resident_key(&self, code_hash: &B256) -> Option<CompiledModuleCacheKey> {
        self.module_keys_by_code_hash
            .get(code_hash)
            .and_then(|profiles| profiles.last().copied())
    }

    /// Makes `key` the profile that hash-only lookups resolve to for its code hash.
    fn index_key(&mut self, key: CompiledModuleCacheKey) {
        let profiles = self
            .module_keys_by_code_hash
            .entry(key.code_hash)
            .or_default();
        profiles.retain(|profile| *profile != key);
        profiles.push(key);
    }

    /// Drops `key` from the index.
    ///
    /// The same code hash can have several resident profiles. Forgetting one keeps the others
    /// reachable by hash, so the hash entry goes away only with its last profile.
    fn forget_key(&mut self, key: &CompiledModuleCacheKey) {
        let Some(profiles) = self.module_keys_by_code_hash.get_mut(&key.code_hash) else {
            return;
        };
        profiles.retain(|profile| profile != key);
        if profiles.is_empty() {
            self.module_keys_by_code_hash.remove(&key.code_hash);
        }
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
        self.index_key(key);
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
        // The replaced profile becomes the hash target, as a fresh insert does.
        self.index_key(*old_key);

        true
    }

    /// Updates size tracking and the code-hash index after an entry is removed.
    fn on_removed(&mut self, key: &mut CompiledModuleCacheKey, value: &mut V) {
        let size = value.estimate_size();
        self.current_bytes = self.current_bytes.saturating_sub(size);
        self.forget_key(key);
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

    /// Counts every profile the code-hash index holds across all hashes.
    fn indexed_profiles(
        cache: &LruMap<CompiledModuleCacheKey, RwasmModule, ModuleMemoryLimiter<RwasmModule>>,
    ) -> usize {
        cache
            .limiter()
            .module_keys_by_code_hash
            .values()
            .map(Vec::len)
            .sum()
    }

    /// Creates a factory with its own cache of `max_bytes`, detached from the global one.
    fn new_factory(max_bytes: usize) -> ModuleFactory {
        ModuleFactory {
            inner: Arc::new(Mutex::new(ModuleFactoryInner {
                cached_modules: new_cache(max_bytes),
            })),
        }
    }

    /// Index cardinality must follow residency, not the number of hashes ever seen.
    #[test]
    fn factory_index_is_bounded_under_deployment_churn() {
        let mut factory = new_factory(500);

        for id in 0..1000 {
            factory.get_or_insert_module(module(100), key(id), Address::ZERO);
            let ctx = factory.inner.lock().unwrap();
            assert!(ctx.cached_modules.len() <= 5);
            assert_eq!(
                indexed_profiles(&ctx.cached_modules),
                ctx.cached_modules.len()
            );
        }
    }

    /// A hash resolves while its module is resident and stops resolving once it is evicted.
    #[test]
    fn hash_only_lookups_follow_residency_across_eviction() {
        let mut factory = new_factory(100);

        assert!(factory.get_resident_module(key(1)).is_none());
        for id in [1, 2, 1] {
            factory.get_or_insert_module(module(100), key(id), Address::ZERO);
            let cached = factory.get_resident_module(key(id)).unwrap();
            assert_eq!(cached.hint_section.len(), 100);
            assert!(factory.get_resident_module(key(3 - id)).is_none());
        }
    }

    /// Evicting the profile a hash resolves to must fall back to another resident profile of
    /// the same code hash instead of making it unreachable.
    #[test]
    fn evicting_indexed_profile_falls_back_to_remaining_resident_profile() {
        let mut factory = new_factory(200);
        let code_hash = key(42);
        let (addr_a, addr_b) = (Address::repeat_byte(0xaa), Address::repeat_byte(0xbb));
        let (key_a, key_b) = (
            module_key_for(code_hash, addr_a),
            module_key_for(code_hash, addr_b),
        );

        factory.get_or_insert_module(module(100), code_hash, addr_a);
        factory.get_or_insert_module(module(100), code_hash, addr_b);
        // Touch profile A so the next eviction takes profile B, the one lookups resolve to.
        factory.get_or_insert_module(module(100), code_hash, addr_a);
        factory.get_or_insert_module(module(100), key(1), Address::ZERO);

        {
            let ctx = factory.inner.lock().unwrap();
            assert!(ctx.cached_modules.peek(&key_a).is_some());
            assert!(ctx.cached_modules.peek(&key_b).is_none());
            assert_eq!(
                ctx.cached_modules.limiter().resident_key(&code_hash),
                Some(key_a)
            );
            assert_eq!(indexed_profiles(&ctx.cached_modules), 2);
        }
        let cached = factory.get_resident_module(code_hash).unwrap();
        assert_eq!(cached.hint_section.len(), 100);
    }

    /// Modules the limiter refuses are still returned but leave no trace in the index.
    #[test]
    fn rejected_modules_are_returned_without_index_entries() {
        let mut factory = new_factory(100);

        for hint_size in [0, 101] {
            let uncached = factory.get_or_insert_module(module(hint_size), key(1), Address::ZERO);
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

    /// Evicting an older profile keeps the newer, still-resident profile as the hash target.
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
        assert_eq!(cache.limiter().resident_key(&code_hash), Some(newer));
        cache.remove(&newer);
        assert!(cache.limiter().resident_key(&code_hash).is_none());
        // The unrelated module stays indexed; only this code hash must be gone.
        assert!(!cache
            .limiter()
            .module_keys_by_code_hash
            .contains_key(&code_hash));
    }

    /// Removing the hash target falls back to the remaining profile; the hash entry goes away
    /// only with the last one.
    #[test]
    fn removing_indexed_profile_falls_back_to_remaining_profile() {
        let mut cache = new_cache(200);
        let code_hash = key(42);
        let key_a = cache_key_with_address_byte(code_hash, 0xaa);
        let key_b = cache_key_with_address_byte(code_hash, 0xbb);

        cache.insert(key_a, module(100));
        cache.insert(key_b, module(100));
        assert_eq!(cache.limiter().resident_key(&code_hash), Some(key_b));

        cache.remove(&key_b);
        assert_eq!(cache.limiter().resident_key(&code_hash), Some(key_a));

        cache.remove(&key_a);
        assert!(cache.limiter().resident_key(&code_hash).is_none());
        assert!(cache.limiter().module_keys_by_code_hash.is_empty());
    }

    /// Replacing a profile's value makes it the hash target again without duplicating it.
    #[test]
    fn replacing_a_profile_makes_it_the_hash_target() {
        let mut cache = new_cache(200);
        let code_hash = key(42);
        let key_a = cache_key_with_address_byte(code_hash, 0xaa);
        let key_b = cache_key_with_address_byte(code_hash, 0xbb);

        cache.insert(key_a, module(100));
        cache.insert(key_b, module(100));
        assert_eq!(cache.limiter().resident_key(&code_hash), Some(key_b));

        cache.insert(key_a, module(50));
        assert_eq!(cache.limiter().resident_key(&code_hash), Some(key_a));
        assert_eq!(indexed_profiles(&cache), 2);
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
