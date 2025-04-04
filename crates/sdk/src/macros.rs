#[macro_export]
macro_rules! debug_log {
    ($msg:tt) => {{
        #[cfg(target_arch = "wasm32")]
        unsafe { $crate::rwasm::_debug_log($msg.as_ptr(), $msg.len() as u32) }
        #[cfg(feature = "std")]
        println!("{}", $msg);
    }};
    ($($arg:tt)*) => {{
        let msg = alloc::format!($($arg)*);
        debug_log!(msg);
    }};
}

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
