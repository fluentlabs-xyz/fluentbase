use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};

#[no_mangle]
pub fn other_deploy_contract_test() -> () {
    let bytecode_to_deploy: &[u8] =
        [126, 194, 144, 82, 202, 235, 49, 22, 55, 154, 163, 43].as_slice();

    LowLevelSDK::sys_write(bytecode_to_deploy);
}
