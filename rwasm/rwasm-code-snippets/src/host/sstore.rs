use crate::common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT};
use fluentbase_sdk::{EvmPlatformSDK, SDK};

#[no_mangle]
pub fn host_sstore() {
    let v = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let k = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    SDK::evm_sstore(&k, &v);
}
