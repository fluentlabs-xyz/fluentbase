#[macro_export]
macro_rules! define_heap_base_allocator {
    () => {
        #[cfg(target_arch = "wasm32")]
        #[global_allocator]
        static ALLOCATOR: $crate::HeapBaseAllocator = $crate::HeapBaseAllocator {};
    };
}

#[macro_export]
macro_rules! define_block_list_allocator {
    () => {
        #[cfg(target_arch = "wasm32")]
        #[global_allocator]
        static ALLOCATOR: $crate::BlockListAllocator = $crate::BlockListAllocator {};
    };
}

#[macro_export]
macro_rules! define_panic_handler {
    () => {
        #[cfg(target_arch = "wasm32")]
        #[panic_handler]
        #[inline(always)]
        unsafe fn panic(info: &core::panic::PanicInfo) -> ! {
            $crate::panic::handle_panic_info(info)
        }
    };
}
