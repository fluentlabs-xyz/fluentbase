#[macro_export]
macro_rules! basic_entrypoint {
    ($struct_typ:ty) => {
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn deploy() {
            let typ = <$struct_typ as Default>::default();
            typ.deploy::<fluentbase_sdk::LowLevelSDK>();
        }
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        extern "C" fn main() {
            let typ = <$struct_typ as Default>::default();
            typ.main::<fluentbase_sdk::LowLevelSDK>();
        }
    };
}
