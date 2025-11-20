#[macro_export]
macro_rules! define_allocator {
    () => {
        #[cfg(target_arch = "wasm32")]
        #[global_allocator]
        static ALLOCATOR: $crate::HeapBaseAllocator = $crate::HeapBaseAllocator {};
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
