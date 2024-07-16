use crate::utils::{evm_builder_apply_envs, fill_eth_tx_env};
use alloy_rlp::{Decodable, Encodable};
use fluentbase_sdk::{
    basic_entrypoint,
    contracts::BlendedAPI,
    derive::Contract,
    Bytes,
    ContextReader,
    SharedAPI,
    SovereignAPI,
    U256,
};
use revm::{interpreter::Host, primitives::ResultAndState, Evm};
use zeth_primitives::{
    receipt::Receipt,
    transactions::{ethereum::EthereumTxEssence, Transaction, TxEssence},
};

#[derive(Contract)]
pub struct BLENDED<CTX: ContextReader, SDK: SovereignAPI> {
    ctx: CTX,
    sdk: SDK,
}

impl<CTX: ContextReader, SDK: SovereignAPI> BlendedAPI for BLENDED<CTX, SDK> {
    fn exec_evm_tx(&self, raw_evm_tx: Bytes) {
        let mut raw_evm_tx = raw_evm_tx.clone();
        let tx = <Transaction<EthereumTxEssence> as Decodable>::decode(&mut raw_evm_tx.as_ref())
            .expect("failed to decode transaction");
        let tx_from = tx.recover_from().expect("failed to recover tx_from");
        let mut evm = evm_builder_apply_envs(Evm::builder(), &self.ctx).build();
        fill_eth_tx_env(&mut evm.context.env_mut().tx, &tx.essence, tx_from);
        let ResultAndState { result, .. } = evm.transact().expect("failed to exec transaction");
        let receipt = Receipt::new(
            tx.essence.tx_type(),
            result.is_success(),
            U256::from(result.gas_used()),
            result
                .logs()
                .into_iter()
                .map(|log| log.clone().into())
                .collect(),
        );
        let receipt_encoded = alloy_rlp::encode(receipt);
        self.sdk.write(&receipt_encoded);
    }

    fn exec_svm_tx(&self, raw_svm_tx: Bytes) {
        todo!("implement svm tx")
    }
}

impl<CTX: ContextReader, SDK: SovereignAPI> BLENDED<CTX, SDK> {
    pub fn deploy(&self) {
        unreachable!("precompiles can't be deployed, it exists since a genesis state")
    }

    pub fn main(&self) {}
}

basic_entrypoint!(BLENDED);
