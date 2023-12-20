use fluentbase_sdk::{EvmPlatformSDK, SDK};

#[no_mangle]
fn system_address() -> [u8; 20] {
    let mut res = [0u8; 20];
    let v = SDK::evm_address();
    res.copy_from_slice(v.as_slice());
    res
}
