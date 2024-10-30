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
            app.sdk.commit_changes_and_exit();
        }
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn main() {
            use fluentbase_sdk::{rwasm::RwasmContext, shared::SharedContextImpl};
            let mut sdk = SharedContextImpl::new(RwasmContext {});
            let mut app = $struct_typ::new(sdk);
            app.main();
            app.sdk.commit_changes_and_exit();
        }
    };
}
