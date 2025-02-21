use crate::context::{BlockContextReader, ContractContextReader, TxContextReader};
use alloc::vec;
use alloy_primitives::{Address, Bytes, U256};
use fluentbase_codec::Codec;

#[derive(Default, Codec, Clone, Debug)]
pub struct ContractContextV1 {
    pub address: Address,
    pub bytecode_address: Address,
    pub caller: Address,
    pub is_static: bool,
    pub value: U256,
}

impl ContractContextReader for ContractContextV1 {
    fn contract_address(&self) -> Address {
        self.address
    }

    fn contract_bytecode_address(&self) -> Address {
        self.bytecode_address
    }

    fn contract_caller(&self) -> Address {
        self.caller
    }

    fn contract_is_static(&self) -> bool {
        self.is_static
    }

    fn contract_value(&self) -> U256 {
        self.value
    }
}

pub fn env_from_context<CR: BlockContextReader + TxContextReader>(cr: CR) -> revm_primitives::Env {
    use revm_primitives::{AnalysisKind, BlockEnv, CfgEnv, Env, TransactTo, TxEnv};
    Env {
        cfg: {
            let mut cfg_env = CfgEnv::default();
            cfg_env.chain_id = cr.block_chain_id();
            cfg_env.perf_analyse_created_bytecodes = AnalysisKind::Raw;
            cfg_env
        },
        block: BlockEnv {
            number: U256::from(cr.block_number()),
            coinbase: cr.block_coinbase(),
            timestamp: U256::from(cr.block_timestamp()),
            gas_limit: U256::from(cr.block_gas_limit()),
            basefee: cr.block_base_fee(),
            difficulty: cr.block_difficulty(),
            prevrandao: Some(cr.block_prev_randao()),
            blob_excess_gas_and_price: None,
        },
        tx: TxEnv {
            caller: cr.tx_origin(),
            gas_limit: cr.tx_gas_limit(),
            gas_price: cr.tx_gas_price(),
            // we don't check this field, and we don't know what type of "transact"
            // we execute right now, so can safely skip the field
            transact_to: TransactTo::Call(Address::ZERO),
            value: cr.tx_value(),
            // we don't use this field, so there is no need to do redundant copy operation
            data: Bytes::default(),
            // we do nonce and chain id checks before executing transaction
            nonce: None,
            chain_id: None,
            // we check access lists in advance before executing a smart contract, it
            // doesn't affect gas price or something else, can skip
            access_list: Default::default(),
            gas_priority_fee: cr.tx_gas_priority_fee(),
            // TODO(dmitry123): "we don't support blobs yet, so 2 tests from e2e testing suite fail"
            blob_hashes: vec![],        // tx_context.blob_hashes.clone(),
            max_fee_per_blob_gas: None, // tx_context.max_fee_per_blob_gas,
            authorization_list: None,
            #[cfg(feature = "optimism")]
            optimism: Default::default(),
        },
    }
}
