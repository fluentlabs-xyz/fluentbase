use fluentbase_sdk::{Bytes, ContextReader, SharedAPI, B256, PROTECTED_STORAGE_SLOT_0};

const SLOT: B256 = PROTECTED_STORAGE_SLOT_0;

pub fn write_protected_preimage(sdk: &mut impl SharedAPI, preimage: Bytes) -> B256 {
    let result = sdk.write_preimage(preimage);
    let code_hash = result.data;
    let result = sdk.write_storage(SLOT.into(), code_hash.into());
    assert!(
        result.status.is_ok(),
        "write_protected_preimage failed with error"
    );
    code_hash
}
pub fn read_protected_preimage(sdk: &impl SharedAPI) -> Bytes {
    let bytecode_address = sdk.context().contract_bytecode_address();
    let code_hash: B256 = sdk
        .delegated_storage(&bytecode_address, &SLOT.into())
        .data
        .0
        .into();
    sdk.preimage(&code_hash)
}
