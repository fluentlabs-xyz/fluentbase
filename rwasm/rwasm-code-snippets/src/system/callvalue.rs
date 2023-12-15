use fluentbase_sdk::{EvmPlatformSDK, SDK};

#[no_mangle]
fn callvalue() -> [u8; 32] {
    let mut res = [0u8; 32];
    let v = SDK::evm_callvalue();
    res.copy_from_slice(&v.to_be_bytes::<32>());
    res
}
