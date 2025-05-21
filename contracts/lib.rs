pub struct GenesisContract {
    pub wasm_bytecode: &'static [u8],
    pub rwasm_bytecode: &'static [u8],
    pub rwasm_bytecode_hash: [u8; 32],
}

pub const BLAKE2F: GenesisContract = GenesisContract {
    wasm_bytecode: fluentbase_contracts_blake2f::WASM_BYTECODE,
    rwasm_bytecode: fluentbase_contracts_blake2f::RWASM_BYTECODE,
    rwasm_bytecode_hash: fluentbase_contracts_blake2f::RWASM_BYTECODE_HASH,
};

pub const EVM: GenesisContract = GenesisContract {
    wasm_bytecode: fluentbase_contracts_blake2f::WASM_BYTECODE,
    rwasm_bytecode: fluentbase_contracts_blake2f::RWASM_BYTECODE,
    rwasm_bytecode_hash: fluentbase_contracts_blake2f::RWASM_BYTECODE_HASH,
};
