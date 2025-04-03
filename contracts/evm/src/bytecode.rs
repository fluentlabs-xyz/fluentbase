use fluentbase_sdk::{
    Bytes,
    ContractContextReader,
    SharedAPI,
    SyscallResult,
    EVM_CODE_HASH_SLOT,
    KECCAK_EMPTY,
    U256,
};

/// Commits EVM bytecode to persistent storage and updates the corresponding code hash.
///
/// This function performs the following operations:
/// 1. Write the provided bytecode (`evm_bytecode`) to preimage storage using the SDK, which returns
///    a hash of the preimage.
/// 2. Takes the resulting code hash and writes it to a predefined storage slot identified by
///    `EVM_CODE_HASH_SLOT`.
///
/// # Arguments
/// - `sdk`: A mutable reference to the SDK instance implementing the `SharedAPI` trait, which
///   provides the methods required for interactions with storage.
/// - `evm_bytecode`: A `Bytes` object containing the EVM bytecode to be stored.
pub(crate) fn commit_evm_bytecode<SDK: SharedAPI>(sdk: &mut SDK, evm_bytecode: Bytes) {
    // TODO(dmitry123): "here we can store pre-analyzed bytecode"
    let result = sdk.write_preimage(evm_bytecode);
    let code_hash = result.data;
    // TODO(dmitry123): "instead of protected slots use metadata storage"
    _ = sdk.write_storage(EVM_CODE_HASH_SLOT.into(), code_hash.into());
}

/// Loads the EVM bytecode associated with the contract using the provided SDK.
///
/// This function retrieves the EVM bytecode for a contract from the state storage
/// using a delegated storage mechanism. The process involves fetching the contract's
/// bytecode address, locating the storage slot for its EVM code hash, and verifying
/// if the bytecode exists (i.e., it is not empty). If valid bytecode is found, it is loaded
/// and returned as a `Bytecode` object.
///
/// # Arguments
/// - `sdk`: A reference to an implementation of the `SharedAPI` trait that provides access to
///   storage, context, and pre-image retrieval methods required for handling contract data.
///
/// # Returns
/// An `Option<Bytecode>`.
/// - `Some(Bytecode)`: If valid bytecode exists and is successfully retrieved.
/// - `None`: If the bytecode is empty or not present in the storage.
pub(crate) fn load_evm_bytecode<SDK: SharedAPI>(sdk: &SDK) -> Option<Bytes> {
    let bytecode_address = sdk.context().contract_bytecode_address();
    let evm_code_hash =
        sdk.delegated_storage(&bytecode_address, &Into::<U256>::into(EVM_CODE_HASH_SLOT));
    let (evm_code_hash, _, _) = evm_code_hash.data;
    // TODO(dmitry123): "do we want to have this optimized during the creation of the frame?"
    let is_empty_bytecode =
        evm_code_hash == U256::ZERO || evm_code_hash == Into::<U256>::into(KECCAK_EMPTY);
    if is_empty_bytecode {
        return None;
    }
    // TODO(dmitry123): "instead of preimages use metadata storage"
    let evm_bytecode = sdk.preimage_copy(&evm_code_hash.into());
    assert!(
        SyscallResult::is_ok(evm_bytecode.status),
        "sdk: failed reading evm bytecode"
    );
    let evm_bytecode = evm_bytecode.data;
    Some(evm_bytecode)
}
