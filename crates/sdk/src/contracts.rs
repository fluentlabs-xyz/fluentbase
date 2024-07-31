use crate::{
    alloc_slice,
    types::{EvmCallMethodInput, EvmCallMethodOutput, EvmCreateMethodInput, EvmCreateMethodOutput},
    NativeAPI,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    U256,
};
use fluentbase_codec_derive::Codec;
use fluentbase_sdk_derive::{client, signature};
pub use fluentbase_types::contracts::*;
use fluentbase_types::{address, Address, Bytes, SovereignAPI};

#[derive(Default, Codec)]
pub struct EvmSloadInput {
    pub index: U256,
}
#[derive(Default, Codec)]
pub struct EvmSloadOutput {
    pub value: U256,
}

#[derive(Default, Codec)]
pub struct EvmSstoreInput {
    pub index: U256,
    pub value: U256,
}
#[derive(Default, Codec)]
pub struct EvmSstoreOutput {}

#[client(mode = "codec")]
pub trait EvmAPI {
    #[signature("_evm_call(address,uint256,bytes,uint64)")]
    fn call(&mut self, input: EvmCallMethodInput) -> EvmCallMethodOutput;

    #[signature("_evm_create(bytes,uint256,u64,bool,uint256)")]
    fn create(&mut self, input: EvmCreateMethodInput) -> EvmCreateMethodOutput;

    #[signature("_evm_sload(uint256)")]
    fn sload(&mut self, input: EvmSloadInput) -> EvmSloadOutput;

    #[signature("_evm_sstore(uint256,uint256)")]
    fn sstore(&mut self, input: EvmSstoreInput) -> EvmSstoreOutput;
}

pub trait WasmAPI {}

pub trait SvmAPI {}

pub trait BlendedAPI {
    fn exec_evm_tx(&self, raw_evm_tx: Bytes);
    fn exec_svm_tx(&self, raw_svm_tx: Bytes);
}
