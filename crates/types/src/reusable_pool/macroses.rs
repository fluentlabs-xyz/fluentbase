#[macro_export]
macro_rules! define_global_reusable_pool {
    ($scope: ident, $item_typ: ty, $keep:expr, $create_strategy: expr, $reset_strategy: expr $(,)?) => {
        pub mod $scope {
            use core::sync::atomic::AtomicUsize;
            pub static CREATED: AtomicUsize = AtomicUsize::new(0);
            pub static REUSED: AtomicUsize = AtomicUsize::new(0);
            pub static RECYCLED: AtomicUsize = AtomicUsize::new(0);
            pub const KEEP: usize = $keep;
            pub type ItemType = $item_typ;
            pub type PoolType = $crate::reusable_pool::ReusablePool<
                ItemType,
                fn() -> $item_typ,
                fn(&mut $item_typ) -> bool,
            >;
            pub static GLOBAL: spin::Once<spin::Mutex<PoolType>> = spin::Once::new();
            pub fn lock<'a>() -> spin::MutexGuard<'a, PoolType> {
                let pool = GLOBAL.call_once(|| {
                    spin::Mutex::new(PoolType::new(
                        $crate::reusable_pool::ReusablePoolConfig::new(
                            KEEP,
                            $create_strategy,
                            $reset_strategy,
                        ),
                    ))
                });
                pool.lock()
            }
            pub fn len() -> usize {
                lock().len()
            }
            pub fn cap() -> usize {
                lock().cap()
            }
            pub fn reuse_or_new() -> ItemType {
                if len() <= 0 {
                    CREATED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                } else {
                    REUSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                lock().reuse_or_new()
            }
            pub fn recycle(item: ItemType) {
                lock().recycle(item);
                RECYCLED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            pub fn take_recycle(item: &mut ItemType) {
                recycle(core::mem::take(item))
            }
            pub fn with_pool_item<F: FnOnce(&mut ItemType)>(item_transformer: F) -> ItemType {
                let mut item = reuse_or_new();
                item_transformer(&mut item);
                item
            }
            pub fn on_pool_item<F: FnOnce(&mut ItemType)>(f: F) {
                let mut item = reuse_or_new();
                f(&mut item);
                recycle(item)
            }
        }
    };
}
