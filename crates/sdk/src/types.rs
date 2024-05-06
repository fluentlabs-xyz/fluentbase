use alloc::vec::Vec;
use fluentbase_codec::Encoder;
use fluentbase_codec_derive::{derive_keccak256_id, Codec};
use fluentbase_types::{Address, Bytes, ExitCode, U256};

#[derive(Default, Debug, Clone, Codec)]
pub struct CoreInput<T: Encoder<T> + Default> {
    pub method_id: u32,
    pub method_data: T,
}

impl<T: Encoder<T> + Default> CoreInput<T> {
    pub fn new(method_id: u32, method_data: T) -> Self {
        CoreInput {
            method_id,
            method_data,
        }
    }
}

pub const EVM_CREATE_METHOD_ID: u32 =
    derive_keccak256_id!("_evm_create(bytes,uint256,u64,bool,uint256)");

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
    pub output: Bytes,
    pub exit_code: i32,
    pub gas: u64,
}

impl EvmCallMethodOutput {
    pub fn from_exit_code(exit_code: ExitCode) -> Self {
        Self {
            output: Default::default(),
            exit_code: exit_code.into_i32(),
            gas: 0,
        }
    }

    pub fn with_output(mut self, output: Bytes) -> Self {
        self.output = output;
        self
    }

    pub fn with_gas(mut self, gas: u64) -> Self {
        self.gas = gas;
        self
    }
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

pub type WasmCallMethodInput = EvmCallMethodInput;
pub type WasmCallMethodOutput = EvmCallMethodOutput;
