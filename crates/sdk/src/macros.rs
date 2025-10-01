#[macro_export]
macro_rules! this_function_path {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            core::any::type_name::<T>()
        }
        let name = type_name_of(f);
        name.strip_suffix("::f").unwrap()
    }};
}
#[macro_export]
macro_rules! current_line_info {
    () => {{
        alloc::format!("{}:{}", $crate::this_function_path!(), core::line!())
    }};
}

#[macro_export]
macro_rules! debug_log {
    ($msg:tt) => {{
        #[cfg(target_arch = "wasm32")]
        {
            #[link(wasm_import_module = "fluentbase_v1preview")]
            extern "C" {
                pub fn _debug_log(msg_ptr: *const u8, msg_len: u32);
            }
            unsafe { _debug_log($msg.as_ptr(), $msg.len() as u32) }
        }
        #[cfg(feature = "std")]
        println!("{}", $msg);
    }};
    ($($arg:tt)*) => {{
        let msg = alloc::format!($($arg)*);
        $crate::debug_log!(msg);
    }};
}

#[macro_export]
macro_rules! debug_log_ext {
    () => {{
        $crate::debug_log_ext!("");
    }};
    ($msg:tt) => {{
        extern crate alloc;
        let msg = alloc::format!("{}: {}", $crate::current_line_info!(), $msg);
        #[cfg(target_arch = "wasm32")]
        {
            #[link(wasm_import_module = "fluentbase_v1preview")]
            extern "C" {
                pub fn _debug_log(msg_ptr: *const u8, msg_len: u32);
            }
            unsafe { _debug_log(msg.as_ptr(), msg.len() as u32) }
        }
        #[cfg(feature = "std")]
        println!("{}", msg);
    }};
    ($($arg:tt)*) => {{
        extern crate alloc;
        let msg = alloc::format!($($arg)*);
        $crate::debug_log_ext!(msg);
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
