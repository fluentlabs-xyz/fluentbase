use crate::evm::{Address, Bytes, U256};
use fluentbase_codec_derive::{derive_keccak256_id, Codec};

#[derive(Default, Clone, Codec)]
pub struct CoreInput {
    pub method_id: u32,
    pub method_data: Vec<u8>,
}

impl CoreInput {
    pub fn new(method_id: u32, method_data: Vec<u8>) -> Self {
        CoreInput {
            method_id,
            method_data,
        }
    }
}

pub const EVM_CREATE_METHOD_ID: u32 =
    derive_keccak256_id!("_evm_create(bytes,uint256,u32,bool,uint256)");

#[derive(Default, Debug, Clone, Codec)]
pub struct EvmCreateMethodInput {
    pub init_code: Bytes,
    pub value: U256,
    pub gas_limit: u64,
    pub salt: Option<U256>,
}

#[derive(Default, Debug, Clone, Codec)]
pub struct EvmCreateMethodOutput {
    pub address: Address,
}

pub const EVM_CALL_METHOD_ID: u32 =
    derive_keccak256_id!(b"_evm_call(address,uint256,bytes,uint64)");

#[derive(Default, Debug, Clone, Codec)]
pub struct EvmCallMethodInput {
    pub callee: Address,
    pub value: U256,
    pub input: Bytes,
    pub gas_limit: u64,
}

#[derive(Default, Debug, Clone, Codec)]
pub struct EvmCallMethodOutput {
    output: Bytes,
}

pub const WASM_CREATE_METHOD_ID: u32 =
    derive_keccak256_id!("_wasm_create(bytes,uint256,uint64,bool,uint256)");

#[derive(Default, Debug, Clone, Codec)]
pub struct WasmCreateMethodInput {
    pub bytecode: Bytes,
    pub value: U256,
    pub gas_limit: u64,
    pub salt: Option<U256>,
}

#[derive(Default, Debug, Clone, Codec)]
pub struct WasmCreateMethodOutput {
    pub address: Address,
}

pub const WASM_CALL_METHOD_ID: u32 =
    derive_keccak256_id!(b"_wasm_call(bytes,uint256,bytes,uint64)");

#[derive(Default, Debug, Clone, Codec)]
pub struct WasmCallMethodInput {
    pub callee: Address,
    pub value: U256,
    pub input: Bytes,
    pub gas_limit: u64,
}

#[derive(Default, Debug, Clone, Codec)]
pub struct WasmCallMethodOutput {
    pub output: Bytes,
}
