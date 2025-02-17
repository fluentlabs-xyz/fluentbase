#[macro_export]
macro_rules! basic_entrypoint {
    ($struct_typ:ident) => {
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn deploy() {
            use fluentbase_sdk::{rwasm::RwasmContext, shared::SharedContextImpl};
            let mut sdk = SharedContextImpl::new(RwasmContext {});
            let mut app = $struct_typ::new(sdk);
            app.deploy();
        }
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn main() {
            use fluentbase_sdk::{rwasm::RwasmContext, shared::SharedContextImpl};
            let mut sdk = SharedContextImpl::new(RwasmContext {});
            let mut app = $struct_typ::new(sdk);
            app.main();
        }
    };
}

#[macro_export]
macro_rules! func_entrypoint {
    ($main_func:ident, $deploy_func:ident) => {
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
            #[cfg(target_arch = "wasm32")]
            #[no_mangle]
            extern "C" fn main() {
                use fluentbase_sdk::{rwasm::RwasmContext, shared::SharedContextImpl};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __main_entry(sdk);
            }
            #[cfg(target_arch = "wasm32")]
            #[no_mangle]
            extern "C" fn deploy() {
                use fluentbase_sdk::{rwasm::RwasmContext, shared::SharedContextImpl};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __deploy_entry(sdk);
            }
        }
    };
    ($main_func:ident) => {
        mod _fluentbase_entrypoint {
            use fluentbase_sdk::SharedAPI;
            #[inline(always)]
            fn __main_entry(sdk: impl SharedAPI) {
                super::$main_func(sdk);
            }
            #[cfg(target_arch = "wasm32")]
            #[no_mangle]
            extern "C" fn main() {
                use fluentbase_sdk::{rwasm::RwasmContext, shared::SharedContextImpl};
                let sdk = SharedContextImpl::new(RwasmContext {});
                __main_entry(sdk);
            }
            #[cfg(target_arch = "wasm32")]
            #[no_mangle]
            extern "C" fn deploy() {}
        }
    };
}
