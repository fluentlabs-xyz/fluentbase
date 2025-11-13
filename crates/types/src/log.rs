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
macro_rules! log_ext {
    () => {{
        $crate::log_ext!("");
    }};
    ($msg:tt) => {{
        extern crate alloc;
        let msg = alloc::format!("{}: {}", $crate::current_line_info!(), $msg);
        #[cfg(feature = "std")]
        println!("{}", msg);
    }};
    ($($arg:tt)*) => {{
        extern crate alloc;
        let msg = alloc::format!($($arg)*);
        $crate::log_ext!(msg);
    }};
}
