use crate::metrics;
use fluentbase_types::{BytecodeOrHash, B256};
use rwasm::RwasmModule;
use schnellru::{Limiter, LruMap};
use std::{
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

    /// Creates a factory with its own cache instead of the process-wide one.
    #[cfg(test)]
    pub(crate) fn isolated(max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ModuleFactoryInner::with_size_limit(max_bytes))),
        }
    }

    /// Returns the cached module for `bytecode_or_hash`, inserting a supplied module on first use.
    ///
    /// The cache is keyed by code hash alone. A cached module is the parsed form of the rWASM
    /// bytes that hash commits to, so one hash names one module at every address; the address
    /// only decides which runtime the executor wraps the module in. Keying by hash keeps a
    /// hash-only lookup a plain table read and leaves no secondary index whose size could
    /// outlive residency.
    ///
    /// Returns `None` only for a hash-only lookup whose module is not resident, either because
    /// nothing warmed it up or because the LRU evicted it. That is a node-local condition, not a
    /// property of the input, so the caller must fail the execution as a host fault rather than
    /// as a contract revert. Nothing in here panics: this lock is shared by every execution
    /// thread, and a panic while holding it would poison it for all of them.
    pub fn get_module_or_init(&mut self, bytecode_or_hash: BytecodeOrHash) -> Option<RwasmModule> {
        let mut ctx = self.lock_cache();
        let code_hash = bytecode_or_hash.code_hash();
        if let Some(entry) = ctx.cached_modules.get(&code_hash) {
            return Some(entry.clone());
        }
        let BytecodeOrHash::Bytecode { bytecode, .. } = bytecode_or_hash else {
            metrics::record_module_cache_hash_miss();
            return None;
        };
        ctx.cached_modules.insert(code_hash, bytecode.clone());
        Some(bytecode)
    }

    /// Locks the cache, recovering from a poisoned lock by discarding the cached contents.
    ///
    /// A poisoned lock means some other thread panicked while holding it. The cache is pure
    /// (bytecode in, compiled module out), so throwing it away is always safe and only costs
    /// recompilation. Propagating the poison instead would make every later execution on every
    /// thread fail at this lock, leaving the node alive but unable to execute anything.
    fn lock_cache(&self) -> std::sync::MutexGuard<'_, ModuleFactoryInner> {
        self.inner.lock().unwrap_or_else(|poisoned| {
            let mut guard = poisoned.into_inner();
            *guard = ModuleFactoryInner::default();
            // `into_inner` leaves the poison flag set; without clearing it every later lock
            // would land here and discard the cache again.
            self.inner.clear_poison();
            metrics::record_module_cache_reset();
            guard
        })
    }
}

struct ModuleFactoryInner {
    pub cached_modules: LruMap<B256, RwasmModule, ModuleMemoryLimiter<RwasmModule>>,
}

/// Maximum memory for module cache: 1 GB
///
/// This limits only the estimated size of cached module content,
/// not the hash table overhead (which is negligible for typical workloads).
/// The table is the only structure keyed by code hash, so nothing beside it can outgrow it.
pub const CACHED_MODULES_SIZE_LIMIT: usize = 1024 * 1024 * 1024;

impl ModuleFactoryInner {
    fn with_size_limit(max_bytes: usize) -> Self {
        Self {
            cached_modules: LruMap::new(ModuleMemoryLimiter::<RwasmModule>::new(max_bytes)),
        }
    }
}

impl Default for ModuleFactoryInner {
    fn default() -> Self {
        Self::with_size_limit(CACHED_MODULES_SIZE_LIMIT)
    }
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

impl<V: SizeEstimator> Limiter<B256, V> for ModuleMemoryLimiter<V> {
    type KeyToInsert<'a> = B256;
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
    ) -> Option<(B256, V)> {
        let size = value.estimate_size();

        if size == 0 || size > self.max_bytes {
            return None;
        }

        self.current_bytes = self.current_bytes.saturating_add(size);
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
        _old_key: &mut B256,
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

        true
    }

    /// Updates size tracking after an entry is removed.
    fn on_removed(&mut self, _key: &mut B256, value: &mut V) {
        let size = value.estimate_size();
        self.current_bytes = self.current_bytes.saturating_sub(size);
    }

    /// Resets size tracking when the cache is cleared.
    fn on_cleared(&mut self) {
        self.current_bytes = 0;
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

    /// Cache key for a module with id `id`: the code hash itself.
    fn cache_key(id: u16) -> B256 {
        key(id)
    }

    fn new_cache(max_bytes: usize) -> LruMap<B256, RwasmModule, ModuleMemoryLimiter<RwasmModule>> {
        LruMap::with_seed(ModuleMemoryLimiter::new(max_bytes), TEST_SEED)
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
    }

    #[test]
    fn replacement_with_zero_size_removes_entry() {
        let mut cache = new_cache(100);

        cache.insert(cache_key(1), module(50));
        cache.insert(cache_key(1), module(0));

        assert_eq!(cache.len(), 0);
        assert_eq!(cache.limiter().current_bytes(), 0);
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

    // ==================== Factory Lookups ====================

    fn bytecode(id: u16, hint_size: usize) -> BytecodeOrHash {
        BytecodeOrHash::Bytecode {
            bytecode: module(hint_size),
            hash: key(id),
            address: fluentbase_types::Address::repeat_byte(id as u8),
        }
    }

    #[test]
    fn hash_lookup_without_warmup_misses_instead_of_panicking() {
        let mut factory = ModuleFactory::isolated(1000);

        assert!(factory
            .get_module_or_init(BytecodeOrHash::Hash(key(1)))
            .is_none());
    }

    #[test]
    fn hash_lookup_after_warmup_hits() {
        let mut factory = ModuleFactory::isolated(1000);

        assert!(factory.get_module_or_init(bytecode(1, 100)).is_some());
        assert!(factory
            .get_module_or_init(BytecodeOrHash::Hash(key(1)))
            .is_some());
    }

    #[test]
    fn hash_lookup_after_eviction_misses_until_rewarmed() {
        // Room for a single module: warming the second evicts the first.
        let mut factory = ModuleFactory::isolated(100);
        factory.get_module_or_init(bytecode(1, 100));
        factory.get_module_or_init(bytecode(2, 100));

        assert!(factory
            .get_module_or_init(BytecodeOrHash::Hash(key(1)))
            .is_none());
        assert!(factory
            .get_module_or_init(BytecodeOrHash::Hash(key(2)))
            .is_some());

        // Re-warming with bytecode restores the hash path.
        factory.get_module_or_init(bytecode(1, 100));
        assert!(factory
            .get_module_or_init(BytecodeOrHash::Hash(key(1)))
            .is_some());
    }

    /// One code hash names one parsed module wherever it is deployed, so a second address does
    /// not get a second entry and a hash-only lookup needs no address to find it.
    #[test]
    fn same_code_hash_at_two_addresses_shares_one_entry() {
        let mut factory = ModuleFactory::isolated(1000);
        let code_hash = key(42);

        for address_byte in [0xaa, 0xbb] {
            let cached = factory
                .get_module_or_init(BytecodeOrHash::Bytecode {
                    bytecode: module(100),
                    hash: code_hash,
                    address: fluentbase_types::Address::repeat_byte(address_byte),
                })
                .unwrap();
            assert_eq!(cached.hint_section.len(), 100);
        }

        let ctx = factory.inner.lock().unwrap();
        assert_eq!(ctx.cached_modules.len(), 1);
        assert_eq!(ctx.cached_modules.limiter().current_bytes(), 100);
        drop(ctx);
        assert!(factory
            .get_module_or_init(BytecodeOrHash::Hash(code_hash))
            .is_some());
    }

    /// FLU-1310: the cache's footprint must follow residency, not the number of distinct code
    /// hashes the process has ever executed.
    #[test]
    fn cache_footprint_follows_residency_under_deployment_churn() {
        let mut factory = ModuleFactory::isolated(500);

        for id in 0..1000u16 {
            factory.get_module_or_init(bytecode(id, 100));
            let ctx = factory.inner.lock().unwrap();
            assert!(ctx.cached_modules.len() <= 5, "deployment {id}");
            assert!(ctx.cached_modules.limiter().current_bytes() <= 500);
        }
    }

    #[test]
    fn poisoned_lock_is_recovered_by_discarding_the_cache() {
        let mut factory = ModuleFactory::isolated(1000);
        factory.get_module_or_init(bytecode(1, 100));

        // Poison the lock the way a panicking execution thread would: unwind while holding it.
        let inner = factory.inner.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = inner.lock().unwrap();
            panic!("simulated panic while holding the module cache lock");
        }));
        assert!(inner.is_poisoned());

        // The cache is usable again and its contents were discarded rather than trusted.
        assert!(factory
            .get_module_or_init(BytecodeOrHash::Hash(key(1)))
            .is_none());
        assert!(!inner.is_poisoned(), "recovery must clear the poison flag");

        // Re-warming works and is not thrown away on the next lock.
        assert!(factory.get_module_or_init(bytecode(1, 100)).is_some());
        assert!(factory
            .get_module_or_init(BytecodeOrHash::Hash(key(1)))
            .is_some());
    }
}
