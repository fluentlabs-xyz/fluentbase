#[macro_export]
macro_rules! basic_entrypoint {
    ($struct_typ:ident) => {
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn deploy() {
            use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext};
            let mut sdk = SharedContextImpl::new(RwasmContext {});
            let mut app = $struct_typ::new(sdk);
            app.deploy();
        }
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn main() {
            use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext};
            let mut sdk = SharedContextImpl::new(RwasmContext {});
            let mut app = $struct_typ::new(sdk);
            app.main();
        }
        #[cfg(target_arch = "wasm32")]
        $crate::define_panic_handler!();
        #[cfg(target_arch = "wasm32")]
        $crate::define_allocator!();
        #[cfg(not(target_arch = "wasm32"))]
        pub fn main() {}
    };
}
#[macro_export]
macro_rules! entrypoint_with_storage {
    ($struct_typ:ident) => {
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn deploy() {
            use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext, U256};
            let mut sdk = SharedContextImpl::new(RwasmContext {});
            let mut app = $struct_typ::new(sdk, U256::from(0), 0);
            app.deploy();
        }
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn main() {
            use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext, U256};
            let mut sdk = SharedContextImpl::new(RwasmContext {});
            let mut app = $struct_typ::new(sdk, U256::from(0), 0);
            app.main();
        }
        #[cfg(target_arch = "wasm32")]
        $crate::define_panic_handler!();
        #[cfg(target_arch = "wasm32")]
        $crate::define_allocator!();
        #[cfg(not(target_arch = "wasm32"))]
        pub fn main() {}
    };
}

#[macro_export]
macro_rules! define_entrypoint {
    ($main_func:ident, $deploy_func:ident) => {
        #[cfg(target_arch = "wasm32")]
        mod _fluentbase_entrypoint {
            use fluentbase_sdk::SharedAPI;
            #[inline(always)]
            fn __main_entry(sdk: impl SharedAPI) {
                super::$main_func(sdk);
            }
            #[inline(always)]
            fn __deploy_entry(sdk: impl SharedAPI) {
                super::$deploy_func(sdk);
            }
            #[no_mangle]
            extern "C" fn main() {
                use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __main_entry(sdk);
            }
            #[no_mangle]
            extern "C" fn deploy() {
                use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __deploy_entry(sdk);
            }
        }
    };
    ($main_func:ident) => {
        #[cfg(target_arch = "wasm32")]
        mod _fluentbase_entrypoint {
            use fluentbase_sdk::SharedAPI;
            #[inline(always)]
            fn __main_entry(sdk: impl SharedAPI) {
                super::$main_func(sdk);
            }
            #[no_mangle]
            extern "C" fn main() {
                use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __main_entry(sdk);
            }
            #[no_mangle]
            extern "C" fn deploy() {}
        }
    };
}

#[macro_export]
macro_rules! func_entrypoint {
    ($main_func:ident, $deploy_func:ident) => {
        $crate::define_entrypoint!($main_func, $deploy_func);
        $crate::define_panic_handler!();
        $crate::define_allocator!();
    };
    ($main_func:ident) => {
        $crate::define_entrypoint!($main_func);
        $crate::define_panic_handler!();
        $crate::define_allocator!();
    };
}

#[macro_export]
macro_rules! entrypoint {
    ($main_func:ident, $deploy_func:ident) => {
        $crate::func_entrypoint!($main_func, $deploy_func);
        #[cfg(not(target_arch = "wasm32"))]
        fn main() {}
    };
    ($main_func:ident) => {
        $crate::func_entrypoint!($main_func);
        #[cfg(not(target_arch = "wasm32"))]
        fn main() {}
    };
}

#[deprecated(note = "migrate to `system_entrypoint2`")]
#[macro_export]
macro_rules! system_entrypoint {
    ($main_func:ident, $deploy_func:ident) => {
        #[cfg(target_arch = "wasm32")]
        mod _fluentbase_entrypoint {
            use alloc::vec::Vec;
            use $crate::{byteorder, byteorder::ByteOrder, Bytes, ExitCode, SharedAPI};
            #[inline(always)]
            fn __main_entry(mut sdk: impl SharedAPI) {
                let (output, exit_code) = match super::$main_func(&mut sdk) {
                    Ok(output) => (output, ExitCode::Ok),
                    Err(exit_code) => (Bytes::new(), exit_code),
                };
                let mut exit_code_le: [u8; 4] = [0u8; 4];
                byteorder::LE::write_i32(&mut exit_code_le, exit_code as i32);
                let mut result = Vec::with_capacity(4 + output.len());
                result.extend_from_slice(&exit_code_le);
                result.extend_from_slice(&output);
                sdk.write(&result);
            }
            #[inline(always)]
            fn __deploy_entry(mut sdk: impl SharedAPI) {
                let (output, exit_code) = match super::$deploy_func(&mut sdk) {
                    Ok(output) => (output, ExitCode::Ok),
                    Err(exit_code) => (Bytes::new(), exit_code),
                };
                let mut exit_code_le: [u8; 4] = [0u8; 4];
                byteorder::LE::write_i32(&mut exit_code_le, exit_code as i32);
                let mut result = Vec::with_capacity(4 + output.len());
                result.extend_from_slice(&exit_code_le);
                result.extend_from_slice(&output);
                sdk.write(&result);
            }
            #[no_mangle]
            extern "C" fn main() {
                use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __main_entry(sdk);
            }
            #[no_mangle]
            extern "C" fn deploy() {
                use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __deploy_entry(sdk);
            }
        }
        $crate::define_panic_handler!();
        $crate::define_allocator!();
        #[cfg(not(target_arch = "wasm32"))]
        fn main() {}
    };
    ($main_func:ident) => {
        #[cfg(target_arch = "wasm32")]
        mod _fluentbase_entrypoint {
            use alloc::vec::Vec;
            use $crate::{byteorder, byteorder::ByteOrder, Bytes, ExitCode, SharedAPI};
            #[inline(always)]
            fn __main_entry(mut sdk: impl SharedAPI) {
                let (output, exit_code) = match super::$main_func(&mut sdk) {
                    Ok(output) => (output, ExitCode::Ok),
                    Err(exit_code) => (Bytes::new(), exit_code),
                };
                let mut exit_code_le: [u8; 4] = [0u8; 4];
                byteorder::LE::write_i32(&mut exit_code_le, exit_code as i32);
                let mut result = Vec::with_capacity(4 + output.len());
                result.extend_from_slice(&exit_code_le);
                result.extend_from_slice(&output);
                sdk.write(&result);
            }
            #[no_mangle]
            extern "C" fn main() {
                use fluentbase_sdk::{shared::SharedContextImpl, RwasmContext};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __main_entry(sdk);
            }
            #[no_mangle]
            extern "C" fn deploy() {}
        }
        $crate::define_panic_handler!();
        $crate::define_allocator!();
        #[cfg(not(target_arch = "wasm32"))]
        fn main() {}
    };
}

#[macro_export]
macro_rules! system_entrypoint2 {
    ($main_func:ident, $deploy_func:ident) => {
        #[cfg(target_arch = "wasm32")]
        mod _fluentbase_entrypoint {
            use $crate::{system::SystemContextImpl, RwasmContext};
            #[no_mangle]
            extern "C" fn main() {
                let mut sdk = SystemContextImpl::new(RwasmContext {});
                let result = super::$main_func(&mut sdk);
                sdk.finalize(result);
            }
            #[no_mangle]
            extern "C" fn deploy() {
                let mut sdk = SystemContextImpl::new(RwasmContext {});
                let result = super::$deploy_func(&mut sdk);
                sdk.finalize(result);
            }
        }
        $crate::define_panic_handler!();
        $crate::define_allocator!();
        #[cfg(not(target_arch = "wasm32"))]
        fn main() {}
    };
    ($main_func:ident) => {
        #[cfg(target_arch = "wasm32")]
        mod _fluentbase_entrypoint {
            use $crate::{system::SystemContextImpl, RwasmContext};
            #[no_mangle]
            extern "C" fn main() {
                let mut sdk = SystemContextImpl::new(RwasmContext {});
                let result = super::$main_func(&mut sdk);
                sdk.finalize(result);
            }
            #[no_mangle]
            extern "C" fn deploy() {}
        }
        $crate::define_panic_handler!();
        $crate::define_allocator!();
        #[cfg(not(target_arch = "wasm32"))]
        fn main() {}
    };
}
