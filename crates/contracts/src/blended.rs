use crate::utils::{evm_builder_apply_envs, fill_eth_tx_env};
use alloy_rlp::{Decodable, Encodable};
use fluentbase_sdk::{
    basic_entrypoint,
    contracts::BlendedAPI,
    derive::Contract,
    AccountManager,
    Bytes,
    ContextReader,
    LowLevelSDK,
    SharedAPI,
};
use revm::{interpreter::Host, primitives::ResultAndState, Evm};
use zeth_primitives::{
    receipt::Receipt,
    transactions::{ethereum::EthereumTxEssence, Transaction, TxEssence},
};

#[derive(Contract)]
pub struct BLENDED<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}

impl<'a, CR: ContextReader, AM: AccountManager> BlendedAPI for BLENDED<'a, CR, AM> {
    fn exec_evm_tx(&self, raw_evm_tx: Bytes) {
        let mut raw_evm_tx = raw_evm_tx.clone();
        let tx = <Transaction<EthereumTxEssence> as Decodable>::decode(&mut raw_evm_tx.as_ref());
        let Ok(tx) = tx else {
            panic!("failed to decode transaction")
        };
        let tx_from = tx.recover_from();
        let Ok(tx_from) = tx_from else {
            panic!("failed to recover tx_from")
        };
        let mut evm = evm_builder_apply_envs(Evm::builder(), self.cr).build();
        fill_eth_tx_env(&mut evm.context.env_mut().tx, &tx.essence, tx_from);
        let result_and_state = evm.transact();
        let Ok(result_and_state) = result_and_state else {
            panic!("failed to exec transaction");
        };
        let ResultAndState { result, state } = result_and_state;
        let gas_used = result.gas_used().try_into().unwrap();

        let receipt = Receipt::new(
            tx.essence.tx_type(),
            result.is_success(),
            gas_used,
            result
                .logs()
                .into_iter()
                .map(|log| log.clone().into())
                .collect(),
        );
        let mut receipt_encoded = alloy_rlp::encode(receipt);
        LowLevelSDK::write(receipt_encoded.as_ptr(), receipt_encoded.len() as u32);
    }

    fn exec_svm_tx(&self, raw_svm_tx: Bytes) {
        todo!("implement svm tx")
    }
}

impl<'a, CR: ContextReader, AM: AccountManager> BLENDED<'a, CR, AM> {
    pub fn deploy<SDK: SharedAPI>(&self) {
        unreachable!("precompiles can't be deployed, it exists since a genesis state")
    }

    pub fn main<SDK: SharedAPI>(&self) {}
}

basic_entrypoint!(
    BLENDED<'static, fluentbase_sdk::GuestContextReader, fluentbase_sdk::GuestAccountManager>
);
