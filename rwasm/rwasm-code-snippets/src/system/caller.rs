use fluentbase_sdk::{EvmPlatformSDK, SDK};

#[no_mangle]
fn caller() -> [u8; 20] {
    let mut res = [0u8; 20];
    let v = SDK::evm_caller();
    res.copy_from_slice(v.as_slice());
    res
}
