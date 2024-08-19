use crate::{
    alloc_slice,
    types::{EvmCallMethodInput, EvmCallMethodOutput, EvmCreateMethodInput, EvmCreateMethodOutput},
    LowLevelSDK,
    SharedAPI,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    U256,
};
use fluentbase_codec_derive::Codec;
use fluentbase_sdk_derive::{client, signature};
pub use fluentbase_types::contracts::*;
use fluentbase_types::{address, Address, Bytes, SovereignAPI};

pub trait BlendedAPI {
    fn exec_evm_tx(&self, raw_evm_tx: Bytes);
    fn exec_fuel_tx(&self, raw_fuel_tx: Bytes);
    fn exec_svm_tx(&self, raw_svm_tx: Bytes);
}