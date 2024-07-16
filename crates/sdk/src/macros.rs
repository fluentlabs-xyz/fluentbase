#[macro_export]
macro_rules! basic_entrypoint {
    ($struct_typ:ident) => {
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn deploy() {
            let typ = <$struct_typ<fluentbase_sdk::rwasm::RwasmContext> as Default>::default();
            typ.deploy();
        }
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn main() {
            let typ = <$struct_typ<fluentbase_sdk::rwasm::RwasmContext> as Default>::default();
            typ.main();
        }
    };
}
