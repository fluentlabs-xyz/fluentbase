use crate::common_sp::{stack_pop_u256, SP_BASE_MEM_OFFSET_DEFAULT};
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn host_sstore() {
    let k = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let v = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    LowLevelSDK::statedb_update_storage(&k, &v);
}
