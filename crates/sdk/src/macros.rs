#[macro_export]
macro_rules! basic_entrypoint {
    ($struct_typ:ident) => {
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn deploy() {
            use fluentbase_sdk::{journal::JournalState, rwasm::RwasmContext};
            let mut typ = $struct_typ::new(JournalState::empty(RwasmContext {}));
            typ.deploy();
        }
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn main() {
            use fluentbase_sdk::{journal::JournalState, rwasm::RwasmContext};
            let mut typ = $struct_typ::new(JournalState::empty(RwasmContext {}));
            typ.main();
        }
    };
}
